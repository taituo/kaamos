//! KAAMOS — a procedural 80s night-synth machine.
//! Every sound is synthesized in code: no samples, no plugins.

mod dsp;
mod engine;
mod fx;
mod instruments;
mod music;
mod params;
mod render;
mod tui;
mod web;

#[cfg(feature = "audio")]
mod audio;

use std::io::Write;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Duration;

use params::Shared;

const USAGE: &str = "\
KAAMOS — procedural 80s night-synth machine

LIVE (default)
  kaamos [options]
      Plays forever. Control it from the terminal or from your phone.

  --intensity X   start intensity 0..1 (default 0.3)
  --darkness X    0 = bright/nostalgic, 1 = ominous (default 0.62)
  --bpm N         tempo (default 92)
  --space X       reverb size 0..1        --tape X   VHS/tape wear 0..1
  --volume X      master volume 0..1      --seed N   reproducible music
  --drift         start in auto-drift mode (wanders calm <-> intense)
  --port N        phone/web panel port (default 7878, 0 = off)
  --bind ADDR     web panel bind address (default 0.0.0.0, use 127.0.0.1 for local only)
  --headless      no terminal UI (control via web panel only)
  --stdout        write raw s16le stereo PCM to stdout instead of the sound card
                  e.g.  kaamos --stdout | ffplay -f s16le -ar 48000 -ac 2 -nodisp -
  --rate N        sample rate for --stdout / render (default 48000)

RENDER
  kaamos render [out.wav] [options]
  --seconds N | --minutes N     length (default 180 s)
  --curve \"0:0.1, 60:0.5, 2m:1, 3m:0.2\"   intensity automation (time:value)
  --events \"30:boom, 45:arp=off, 1m:next, 90:darkness=0.9, 2m30:fade\"
  --no-fade                     don't fade out at the end
  (all live options like --seed --darkness --bpm also work)

EVENT / WEB COMMANDS
  triggers: boom fill riser next key        toggles: drift hold fade (or drift=on)
  layers:   drone pad arp bass drums lead bells  =auto|on|off|cycle
  params:   intensity darkness bpm space tape volume drift_min drift_max =value
";

pub struct Opts {
    pub seed: u64,
    pub params: Vec<(String, f32)>,
    pub drift: bool,
    pub port: u16,
    pub bind: String,
    pub headless: bool,
    pub stdout: bool,
    pub rate: Option<u32>,
    pub output: Option<String>,
    pub seconds: Option<f32>,
    pub curve: Option<String>,
    pub events: Option<String>,
    pub no_fade: bool,
}

impl Opts {
    fn parse(args: &[String], render: bool) -> Result<Opts, String> {
        let mut o = Opts {
            seed: time_seed(),
            params: Vec::new(),
            drift: false,
            port: 7878,
            bind: "0.0.0.0".into(),
            headless: false,
            stdout: false,
            rate: None,
            output: None,
            seconds: None,
            curve: None,
            events: None,
            no_fade: false,
        };
        let mut i = 0;
        while i < args.len() {
            let a = args[i].as_str();
            let (flag, inline) = match a.split_once('=') {
                Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
                _ => (a, None),
            };
            let mut val = |name: &str| -> Result<String, String> {
                if let Some(v) = &inline {
                    return Ok(v.clone());
                }
                i += 1;
                args.get(i).cloned().ok_or_else(|| format!("{name} needs a value"))
            };
            let num = |s: String, name: &str| -> Result<f32, String> {
                s.parse::<f32>().map_err(|_| format!("{name}: '{s}' is not a number"))
            };
            match flag {
                "--intensity" | "--darkness" | "--bpm" | "--space" | "--tape" | "--volume" | "--drift-min"
                | "--drift-max" => {
                    let v = num(val(flag)?, flag)?;
                    o.params.push((flag.trim_start_matches("--").replace('-', "_"), v));
                }
                "--seed" => o.seed = val(flag)?.parse().map_err(|_| "--seed needs an integer".to_string())?,
                "--drift" => o.drift = true,
                "--port" => o.port = val(flag)?.parse().map_err(|_| "--port needs a number".to_string())?,
                "--bind" => o.bind = val(flag)?,
                "--headless" => o.headless = true,
                "--stdout" => o.stdout = true,
                "--rate" => o.rate = Some(val(flag)?.parse().map_err(|_| "--rate needs a number".to_string())?),
                "--seconds" => o.seconds = Some(num(val(flag)?, flag)?),
                "--minutes" => o.seconds = Some(num(val(flag)?, flag)? * 60.0),
                "--curve" => o.curve = Some(val(flag)?),
                "--events" => o.events = Some(val(flag)?),
                "--no-fade" => o.no_fade = true,
                _ if render && !a.starts_with("--") && o.output.is_none() => o.output = Some(a.to_string()),
                _ => return Err(format!("unknown option '{a}' (try: kaamos help)")),
            }
            i += 1;
        }
        Ok(o)
    }

    pub fn apply(&self, sh: &Shared) {
        for (k, v) in &self.params {
            sh.set_param(k, *v);
        }
        if self.drift {
            sh.drift.store(true, Relaxed);
        }
    }
}

fn time_seed() -> u64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1);
    (t ^ (t >> 29)) % 1_000_000
}

pub struct LiveInfo {
    pub device: String,
    pub rate: u32,
    pub seed: u64,
    pub url: Option<String>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let res = match args.first().map(String::as_str) {
        Some("render") => Opts::parse(&args[1..], true).and_then(|o| render::run(&o)),
        Some("help") | Some("-h") | Some("--help") => {
            print!("{USAGE}");
            Ok(())
        }
        _ => Opts::parse(&args, false).and_then(|o| live(&o)),
    };
    if let Err(e) = res {
        eprintln!("kaamos: {e}");
        std::process::exit(1);
    }
}

fn live(o: &Opts) -> Result<(), String> {
    let sh = Arc::new(Shared::new());
    o.apply(&sh);

    // keep the audio stream alive for the whole session
    let mut _keep: Vec<Box<dyn std::any::Any>> = Vec::new();

    let (device, rate) = if o.stdout {
        let rate = o.rate.unwrap_or(48000);
        let sh2 = sh.clone();
        let seed = o.seed;
        std::thread::spawn(move || {
            let mut eng = engine::Engine::new(rate as f32, sh2.clone(), seed);
            let out = std::io::stdout();
            let mut lock = out.lock();
            let mut buf = vec![0u8; 2048 * 4];
            loop {
                for f in 0..2048 {
                    let (l, r) = eng.next();
                    let a = (l.clamp(-1.0, 1.0) * 32767.0) as i16;
                    let b = (r.clamp(-1.0, 1.0) * 32767.0) as i16;
                    buf[f * 4..f * 4 + 2].copy_from_slice(&a.to_le_bytes());
                    buf[f * 4 + 2..f * 4 + 4].copy_from_slice(&b.to_le_bytes());
                }
                if lock.write_all(&buf).is_err() {
                    // the player went away: stop the whole program
                    sh2.quitting.store(true, Relaxed);
                    break;
                }
            }
        });
        ("stdout (s16le)".to_string(), rate)
    } else {
        #[cfg(feature = "audio")]
        {
            let (stream, rate, name) = audio::start(sh.clone(), o.seed)?;
            _keep.push(Box::new(stream));
            (name, rate)
        }
        #[cfg(not(feature = "audio"))]
        {
            return Err(
                "this build has no sound-card output (built without the 'audio' feature). \
                        Use --stdout, or 'kaamos render'."
                    .into(),
            );
        }
    };

    let url = if o.port != 0 {
        match web::spawn(sh.clone(), &o.bind, o.port) {
            Ok(u) => Some(u),
            Err(e) => {
                eprintln!("kaamos: web panel disabled: {e}");
                None
            }
        }
    } else {
        None
    };

    let info = LiveInfo {
        device,
        rate,
        seed: o.seed,
        url,
    };

    if o.headless || !tui::available() {
        eprintln!(
            "kaamos: playing on {} @ {} Hz, seed {}",
            info.device, info.rate, info.seed
        );
        if let Some(u) = &info.url {
            eprintln!("kaamos: control panel at {u}");
        }
        eprintln!("kaamos: Ctrl-C to stop");
        loop {
            std::thread::sleep(Duration::from_millis(500));
            if sh.quitting.load(Relaxed) {
                break;
            }
        }
    } else {
        tui::run(&sh, &info)?;
    }

    // short fade so we never end on a click
    sh.quitting.store(true, Relaxed);
    std::thread::sleep(Duration::from_millis(650));
    Ok(())
}
