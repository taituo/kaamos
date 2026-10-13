//! Whole songs: the same generator, arranged into a form.
//!
//! intro (sometimes a radio surprise) → verse that opens with the bass or the lead →
//! pre-chorus build → chorus → verse 2 → chorus 2 → B-section (breakdown + build) →
//! climax (BOOM, often a key change up) → ending (fade, hard stop, or back to the intro).
//!
//! Verse, chorus and bridge each have fixed material, so the chorus is the same hook
//! every time it comes back.

use crate::dsp::Rng;
use crate::music::*;
use crate::params::*;
use crate::radio::{self, Content};

/// Musical material for one part of a song.
#[derive(Clone, Debug)]
pub struct Part {
    pub key: i32,
    pub mode: usize,
    pub prog: Vec<i32>,
    pub bpc: u32,
    pub motif: Vec<usize>,
    pub bass_pats: [usize; 3],
    pub drum_var: u8,
    pub keys_pat: usize,
    pub lead: Vec<LeadNote>,
}

impl Part {
    pub fn loop_bars(&self) -> u32 {
        self.prog.len() as u32 * self.bpc.max(1)
    }

    pub fn transposed(&self, semis: i32) -> Part {
        let mut p = self.clone();
        p.key += semis;
        for n in p.lead.iter_mut() {
            n.midi += semis;
        }
        p
    }
}

pub const P_VERSE: usize = 0;
pub const P_PRE: usize = 1;
pub const P_CHORUS: usize = 2;
pub const P_BRIDGE: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Intro,
    Verse,
    Pre,
    Chorus,
    Bridge,
    Build,
    Climax,
    Outro,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Ev {
    /// the radio passage (intro)
    Radio,
    /// a few seconds of static (callback in the bridge / outro)
    Static(f32),
    /// a riser lasting this many bars, landing on the next section's downbeat
    Riser(u32),
    /// a drum fill in the second half of the bar (0 = toms, 1 = snare build)
    Fill(u8),
    Boom,
    Crash(f32),
    /// final chord hit; the sequencers stop after it
    Hit,
}

#[derive(Clone, Debug)]
pub struct Sec {
    pub name: String,
    pub kind: Kind,
    pub part: usize,
    pub transpose: i32,
    pub bars: u32,
    /// intensity at the first and last bar (ramps in between)
    pub i0: f32,
    pub i1: f32,
    pub drums: u8,
    pub arp_rate: u32,
    pub bass_level: usize,
    /// arrangement: how loud each layer is in this section
    pub gains: [f32; NL],
    /// events at bar offsets within the section
    pub events: Vec<(u32, Ev)>,
    /// the song fades out from this bar to the end of the section
    pub fade_from: Option<u32>,
    /// layers switch crisply on the downbeat instead of gliding
    pub snap: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ending {
    Fade,
    Stop,
    Bookend,
}

impl Ending {
    pub fn parse(s: &str) -> Option<Ending> {
        match s {
            "fade" => Some(Ending::Fade),
            "stop" | "hit" => Some(Ending::Stop),
            "bookend" | "intro" => Some(Ending::Bookend),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Opening {
    Bass,
    Lead,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RadioPref {
    Auto,
    On,
    Off,
}

pub struct SongOpts {
    pub seed: u64,
    pub darkness: f32,
    pub bpm: Option<f32>,
    pub target_secs: Option<f32>,
    pub voice_secs: Vec<f32>,
    pub radio: RadioPref,
    pub ending: Option<Ending>,
}

#[derive(Clone, Debug)]
pub struct Song {
    pub seed: u64,
    pub bpm: f32,
    pub key: i32,
    pub mode: usize,
    pub parts: Vec<Part>,
    pub secs: Vec<Sec>,
    pub radio: Option<Content>,
    pub opening: Opening,
    pub ending: Ending,
}

//                       drone  pad   arp   bass  drums lead  bells keys
const G_INTRO: [f32; NL] = [1.0, 0.65, 0.0, 0.0, 0.0, 0.0, 0.7, 0.0];
const G_INTRO_RADIO: [f32; NL] = [0.7, 0.45, 0.0, 0.0, 0.0, 0.0, 0.4, 0.0];
const G_INTRO_ARP: [f32; NL] = [0.6, 0.6, 1.0, 0.0, 0.0, 0.0, 0.3, 0.0];
const G_OPEN_BASS: [f32; NL] = [0.7, 0.75, 0.0, 1.0, 1.0, 0.0, 0.3, 0.0];
const G_OPEN_LEAD: [f32; NL] = [0.7, 0.8, 0.0, 0.0, 0.0, 1.0, 0.3, 0.0];
const G_VERSE: [f32; NL] = [0.6, 0.85, 0.75, 1.0, 1.0, 1.0, 0.25, 0.45];
const G_PRE: [f32; NL] = [0.5, 1.0, 1.0, 1.0, 1.0, 0.0, 0.2, 0.7];
const G_CHORUS: [f32; NL] = [0.35, 1.0, 1.0, 1.0, 1.0, 1.0, 0.3, 0.5];
const G_VERSE2: [f32; NL] = [0.5, 0.75, 0.5, 0.85, 1.0, 0.85, 0.3, 0.7];
const G_BRIDGE: [f32; NL] = [0.8, 0.75, 0.0, 0.0, 0.0, 0.0, 0.85, 0.85];
const G_BUILD: [f32; NL] = [0.8, 1.0, 1.0, 0.7, 1.0, 0.0, 0.4, 0.8];
const G_CLIMAX: [f32; NL] = [0.3, 1.0, 1.0, 1.0, 1.0, 1.0, 0.2, 0.45];
const G_END: [f32; NL] = [0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0];

/// Which optional parts of the form are in.
#[derive(Clone, Copy)]
struct Form {
    intro: u32,
    verse1b: bool,
    pre1: bool,
    verse2: bool,
    pre2: bool,
    chorus2: u32,
    climax: u32,
}

impl Song {
    pub fn plan(o: &SongOpts) -> Song {
        let mut rng = Rng::new(o.seed ^ 0x534f_4e47);
        let pc = *rng.pick(&[9, 2, 4, 0, 5, 7, 11, 6, 1]);
        let key = key_root(pc);
        let mode = pick_mode(&mut rng, o.darkness);
        let bpm = o
            .bpm
            .unwrap_or_else(|| *rng.pick(&[86.0, 90.0, 92.0, 96.0, 100.0, 104.0, 108.0]));
        let target = o.target_secs.unwrap_or_else(|| rng.range(140.0, 220.0));
        let bar = 240.0 / bpm;

        let distinct = |p: &[i32]| {
            let mut d = p.to_vec();
            d.sort();
            d.dedup();
            d.len()
        };

        // ---- verse: home chord first, unhurried
        let vprog = pick_prog_where(&mut rng, mode, |p| p[0] == 0 && p.len() >= 2).unwrap_or_else(|| vec![0, 0]);
        let vbpc = if vprog.len() == 2 || distinct(&vprog) <= 2 && vprog.len() == 4 && rng.chance(0.3) {
            2
        } else if rng.chance(0.4) {
            2
        } else {
            1
        };
        let verse_motif = arp_motif(&mut rng, 4);
        let verse = Part {
            key,
            mode,
            lead: gen_lead_styled(&mut rng, key, mode, &vprog, vbpc, &VERSE_STYLE),
            prog: vprog.clone(),
            bpc: vbpc,
            motif: verse_motif.clone(),
            bass_pats: [
                rng.below(BASS_LOW.len()),
                rng.below(BASS_MID.len()),
                rng.below(BASS_HIGH.len()),
            ],
            drum_var: rng.below(3) as u8,
            keys_pat: pick_keys_pattern(&mut rng, 0.45),
        };

        // ---- chorus: four chords, usually starting away from home (the lift)
        let lift = rng.chance(0.7);
        let cprog = pick_prog_where(&mut rng, mode, |p| {
            p.len() == 4 && distinct(p) >= 3 && p != vprog.as_slice() && (!lift || p[0] != 0)
        })
        .or_else(|| pick_prog_where(&mut rng, mode, |p| p.len() == 4 && p != vprog.as_slice()))
        .unwrap_or_else(|| vprog.clone());
        let chorus = Part {
            key,
            mode,
            lead: gen_lead_styled(&mut rng, key, mode, &cprog, 1, &CHORUS_STYLE),
            prog: cprog.clone(),
            bpc: 1,
            motif: if rng.chance(0.5) {
                verse_motif.clone()
            } else {
                arp_motif(&mut rng, 4)
            },
            bass_pats: [
                rng.below(BASS_LOW.len()),
                rng.below(BASS_MID.len()),
                rng.below(BASS_HIGH.len()),
            ],
            drum_var: rng.below(3) as u8,
            keys_pat: pick_keys_pattern(&mut rng, 0.85),
        };

        // ---- pre-chorus: two chords that want to go somewhere
        let pre_opts: Vec<[i32; 2]> = [[3, 4], [5, 6], [3, 6], [1, 4], [5, 4], [3, 5], [1, 6]]
            .into_iter()
            .filter(|p| p.iter().all(|&d| is_major_or_minor(mode, d)) && p[1] != cprog[0])
            .collect();
        let pprog = if pre_opts.is_empty() {
            vec![cprog[3], cprog[3]]
        } else {
            pre_opts[rng.below(pre_opts.len())].to_vec()
        };
        let pre = Part {
            key,
            mode,
            prog: pprog,
            bpc: 1,
            motif: chorus.motif.clone(),
            bass_pats: chorus.bass_pats,
            drum_var: verse.drum_var,
            keys_pat: *rng.pick(&[1, 4, 5]),
            lead: Vec::new(),
        };

        // ---- bridge (B-section): somewhere new, carried by the keys and bells
        let bprog = pick_prog_where(&mut rng, mode, |p| {
            p[0] != 0 && p != vprog.as_slice() && p != cprog.as_slice() && p.len() >= 2
        })
        .unwrap_or_else(|| vec![cprog[1], cprog[2]]);
        let bbpc = if bprog.len() == 2 { 2 } else { 1 };
        let bridge = Part {
            key,
            mode,
            prog: bprog,
            bpc: bbpc,
            motif: arp_motif(&mut rng, 4),
            bass_pats: verse.bass_pats,
            drum_var: chorus.drum_var,
            keys_pat: *rng.pick(&[3, 6, 0]),
            lead: Vec::new(),
        };

        // ---- character of this song
        let opening = if rng.chance(0.5) { Opening::Bass } else { Opening::Lead };
        let use_radio = match o.radio {
            RadioPref::On => true,
            RadioPref::Off => false,
            RadioPref::Auto => !o.voice_secs.is_empty() || rng.chance(0.5),
        };
        let radio = use_radio.then(|| {
            if !o.voice_secs.is_empty() {
                Content::Voices
            } else {
                match rng.weighted(&[0.25, 0.2, 0.35, 0.2]) {
                    0 => Content::Morse("KAAMOS".into()),
                    1 => Content::Morse(radio::random_group(&mut rng)),
                    2 => {
                        let mut notes: Vec<i32> = chorus.lead.iter().take(6).map(|n| n.midi).collect();
                        while notes.iter().any(|&n| n < 74) {
                            for n in notes.iter_mut() {
                                *n += 12;
                            }
                        }
                        Content::Signal(notes)
                    }
                    _ => Content::Tuning,
                }
            }
        });
        let arp_intro = radio.is_none() && rng.chance(0.5);
        let intro_part = if rng.chance(0.3) { P_CHORUS } else { P_VERSE };
        let ending = o.ending.unwrap_or_else(|| match rng.weighted(&[0.4, 0.35, 0.25]) {
            0 => Ending::Fade,
            1 => Ending::Stop,
            _ => Ending::Bookend,
        });
        let climax_shift = *rng.pick(&[2, 2, 1, 0, 0]);
        let bridge_static = radio.is_some() && rng.chance(0.5);

        // radio needs room: the intro lasts at least until the content is over
        let radio_bars = if radio.is_some() {
            let content = radio::content_end(radio.as_ref().unwrap(), &o.voice_secs);
            let b = ((content + 1.0) / bar).ceil() as u32;
            b.div_ceil(2) * 2
        } else {
            0
        };

        // ---- fit the form to the target length
        let mut f = Form {
            intro: 8,
            verse1b: true,
            pre1: true,
            verse2: true,
            pre2: true,
            chorus2: 8,
            climax: 16,
        };
        let vl = verse.loop_bars();
        let outro_bars = match ending {
            Ending::Stop => 4,
            _ => 8,
        };
        let total = |f: &Form| -> f32 {
            let intro = f.intro.max(radio_bars);
            let bars = intro
                + 8u32.max(vl)
                + if f.verse1b { 8u32.max(vl) } else { 0 }
                + if f.pre1 { 4 } else { 0 }
                + 8
                + if f.verse2 {
                    8u32.max(vl) + f.chorus2 + if f.pre2 { 4 } else { 0 }
                } else {
                    0
                }
                + 8
                + 4
                + f.climax
                + outro_bars;
            bars as f32 * bar
        };
        let trims: [fn(&mut Form); 6] = [
            |f| f.verse1b = false,
            |f| f.pre2 = false,
            |f| f.climax = 8,
            |f| f.pre1 = false,
            |f| f.verse2 = false,
            |f| f.intro = 4,
        ];
        for t in trims {
            if total(&f) <= target + 8.0 {
                break;
            }
            t(&mut f);
        }
        if total(&f) < target - 20.0 {
            f.chorus2 = 16;
        }
        // never shorter than two minutes: put parts back, last trimmed first
        let restores: [fn(&mut Form); 6] = [
            |f| f.intro = 8,
            |f| f.verse2 = true,
            |f| f.pre1 = true,
            |f| f.climax = 16,
            |f| f.pre2 = true,
            |f| f.verse1b = true,
        ];
        for r in restores {
            if total(&f) >= 125.0 {
                break;
            }
            r(&mut f);
        }

        // ---- write out the sections
        let mut secs: Vec<Sec> = Vec::new();
        let sec = |name: &str, kind: Kind, part: usize, bars: u32, i0: f32, i1: f32, gains: [f32; NL]| Sec {
            name: name.to_string(),
            kind,
            part,
            transpose: 0,
            bars,
            i0,
            i1,
            drums: 1,
            arp_rate: 2,
            bass_level: 1,
            gains,
            events: Vec::new(),
            fade_from: None,
            snap: false,
        };

        // intro
        let intro_bars = f.intro.max(radio_bars);
        let mut s = if arp_intro {
            let mut s = sec("intro", Kind::Intro, intro_part, intro_bars, 0.15, 0.4, G_INTRO_ARP);
            s.arp_rate = 2;
            s
        } else {
            sec("intro", Kind::Intro, intro_part, intro_bars, 0.08, 0.2, G_INTRO)
        };
        if radio.is_some() {
            s.events.push((0, Ev::Radio));
            s.gains = G_INTRO_RADIO; // the music waits underneath while the radio talks
        }
        secs.push(s);

        // verse 1: opens with the bass or the lead
        let v1 = 8u32.max(vl);
        let mut s = match opening {
            Opening::Bass => {
                let mut s = sec("verse 1", Kind::Verse, P_VERSE, v1, 0.42, 0.42, G_OPEN_BASS);
                s.drums = 0; // heartbeat kick under the bass
                s
            }
            Opening::Lead => sec("verse 1", Kind::Verse, P_VERSE, v1, 0.42, 0.42, G_OPEN_LEAD),
        };
        if arp_intro {
            s.gains[L_ARP] = 0.8;
        }
        s.snap = true;
        secs.push(s);
        if f.verse1b {
            let mut s = sec("verse 1b", Kind::Verse, P_VERSE, 8u32.max(vl), 0.5, 0.5, G_VERSE);
            s.snap = true;
            secs.push(s);
        }

        let push_pre = |secs: &mut Vec<Sec>, n: &str, with_pre: bool| {
            if with_pre {
                let mut s = sec(n, Kind::Pre, P_PRE, 4, 0.55, 0.78, G_PRE);
                s.arp_rate = 1;
                s.events = vec![(2, Ev::Riser(2)), (3, Ev::Fill(1))];
                secs.push(s);
            } else if let Some(last) = secs.last_mut() {
                let b = last.bars - 1;
                last.events.push((b, Ev::Riser(1)));
                last.events.push((b, Ev::Fill(0)));
            }
        };
        let chorus_sec = |n: &str, bars: u32, i: f32, drums: u8| {
            let mut s = sec(n, Kind::Chorus, P_CHORUS, bars, i, i, G_CHORUS);
            s.drums = drums;
            s.arp_rate = 1;
            s.snap = true;
            s.events.push((0, Ev::Crash(0.6)));
            if bars >= 16 {
                s.events.push((7, Ev::Fill(1)));
                s.events.push((8, Ev::Crash(0.45)));
            }
            s
        };

        push_pre(&mut secs, "pre-chorus", f.pre1);
        let mut c1 = chorus_sec("chorus 1", 8, 0.82, 2);
        if f.verse2 {
            c1.events.push((7, Ev::Fill(0)));
        }
        secs.push(c1);

        if f.verse2 {
            let mut s = sec("verse 2", Kind::Verse, P_VERSE, 8u32.max(vl), 0.56, 0.56, G_VERSE2);
            s.snap = true;
            secs.push(s);
            push_pre(&mut secs, "pre-chorus 2", f.pre2);
            secs.push(chorus_sec("chorus 2", f.chorus2, 0.88, 2));
        }

        // B-section: breakdown, then a build
        let mut s = sec("B-section", Kind::Bridge, P_BRIDGE, 8, 0.3, 0.3, G_BRIDGE);
        s.snap = true;
        s.arp_rate = 2;
        if bridge_static {
            s.events.push((0, Ev::Static(bar * 3.0)));
        }
        secs.push(s);
        let mut s = sec("build", Kind::Build, P_BRIDGE, 4, 0.35, 0.82, G_BUILD);
        s.arp_rate = 1;
        s.events = vec![(0, Ev::Riser(4)), (3, Ev::Fill(1))];
        secs.push(s);

        // climax
        let mut s = chorus_sec("climax", f.climax, 1.0, 3);
        s.kind = Kind::Climax;
        s.transpose = climax_shift;
        s.gains = G_CLIMAX;
        s.bass_level = 2;
        s.events.retain(|e| e.1 != Ev::Crash(0.6));
        s.events.push((0, Ev::Boom));
        if ending == Ending::Stop {
            let b = s.bars - 1;
            s.events.push((b, Ev::Fill(1)));
        }
        secs.push(s);

        // ending
        match ending {
            Ending::Fade => {
                let mut s = chorus_sec("outro (fade)", 8, 0.95, 3);
                s.kind = Kind::Outro;
                s.transpose = climax_shift;
                s.gains = G_CLIMAX;
                s.bass_level = 2;
                s.events.clear();
                s.fade_from = Some(0);
                s.snap = false;
                secs.push(s);
            }
            Ending::Stop => {
                let mut s = sec("end", Kind::End, P_CHORUS, 4, 1.0, 1.0, G_END);
                s.transpose = climax_shift;
                s.drums = 3;
                s.events = vec![(0, Ev::Hit)];
                s.snap = true;
                s.fade_from = Some(1); // let it ring, then settle to silence
                secs.push(s);
            }
            Ending::Bookend => {
                let mut s = sec("outro", Kind::Outro, intro_part, 8, 0.2, 0.12, G_INTRO);
                s.snap = true;
                if radio.is_some() {
                    s.events.push((2, Ev::Static(bar * 6.0)));
                }
                s.fade_from = Some(4);
                secs.push(s);
            }
        }

        Song {
            seed: o.seed,
            bpm,
            key,
            mode,
            parts: vec![verse, pre, chorus, bridge],
            secs,
            radio,
            opening,
            ending,
        }
    }

    pub fn bar_secs(&self) -> f32 {
        240.0 / self.bpm
    }

    pub fn total_bars(&self) -> u32 {
        self.secs.iter().map(|s| s.bars).sum()
    }

    pub fn total_secs(&self) -> f32 {
        self.total_bars() as f32 * self.bar_secs()
    }

    pub fn key_name(&self) -> String {
        format!(
            "{} {}",
            NOTE_NAMES[self.key.rem_euclid(12) as usize],
            MODES[self.mode].name
        )
    }

    /// A readable plan with timestamps.
    pub fn describe(&self, voices: usize) -> String {
        let ending = match self.ending {
            Ending::Fade => "fade-out ending",
            Ending::Stop => "hard-stop ending",
            Ending::Bookend => "back-to-the-intro ending",
        };
        let mut out = format!(
            "KAAMOS song · seed {} · {} · {:.0} bpm · {} · {ending}\n",
            self.seed,
            self.key_name(),
            self.bpm,
            mmss(self.total_secs())
        );
        let mut t = 0.0;
        let bar = self.bar_secs();
        for s in &self.secs {
            let part = &self.parts[s.part];
            let prog: &[i32] = if s.kind == Kind::End { &[0] } else { &part.prog };
            let chords: Vec<String> = prog
                .iter()
                .map(|&d| chord_name(part.key + s.transpose, part.mode, d, 3))
                .collect();
            let mut notes: Vec<String> = Vec::new();
            match s.kind {
                Kind::Verse if s.name == "verse 1" => notes.push(match self.opening {
                    Opening::Bass => "opens with the bass".into(),
                    Opening::Lead => "opens with the lead".into(),
                }),
                _ => {}
            }
            for (_, e) in &s.events {
                match e {
                    Ev::Radio => notes.push(format!(
                        "radio: {}",
                        self.radio.as_ref().map(|c| c.describe(voices)).unwrap_or_default()
                    )),
                    Ev::Static(_) => notes.push("radio static".into()),
                    Ev::Boom => notes.push("BOOM".into()),
                    Ev::Hit => notes.push("final hit".into()),
                    Ev::Riser(b) if *b >= 2 => notes.push(format!("{b}-bar riser")),
                    _ => {}
                }
            }
            if s.transpose != 0 {
                notes.push(format!("key change +{}", s.transpose));
            }
            if s.fade_from.is_some() {
                notes.push("fade out".into());
            }
            out.push_str(&format!(
                "  {:>5}  {:<13} {:>3} bars  {:<22} {}\n",
                mmss(t),
                s.name,
                s.bars,
                chords.join(" "),
                notes.join(", ")
            ));
            t += s.bars as f32 * bar;
        }
        out
    }
}

pub fn mmss(t: f32) -> String {
    let t = t.max(0.0) as u32;
    format!("{}:{:02}", t / 60, t % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(seed: u64, target: Option<f32>) -> SongOpts {
        SongOpts {
            seed,
            darkness: 0.6,
            bpm: None,
            target_secs: target,
            voice_secs: vec![],
            radio: RadioPref::Auto,
            ending: None,
        }
    }

    #[test]
    fn songs_have_the_form_and_length() {
        for seed in 0..200 {
            let s = Song::plan(&opts(seed, None));
            let t = s.total_secs();
            assert!((120.0..=250.0).contains(&t), "seed {seed}: {t}s");
            let kinds: Vec<Kind> = s.secs.iter().map(|x| x.kind).collect();
            assert_eq!(kinds[0], Kind::Intro);
            assert!(kinds.contains(&Kind::Chorus), "seed {seed}");
            assert!(kinds.contains(&Kind::Bridge));
            assert!(kinds.contains(&Kind::Climax));
            // the chorus material is the same object every time it comes back
            assert!(s
                .secs
                .iter()
                .filter(|x| x.kind == Kind::Chorus)
                .all(|x| x.part == P_CHORUS));
            assert!(!s.parts[P_CHORUS].lead.is_empty());
            assert_ne!(s.parts[P_VERSE].prog, s.parts[P_CHORUS].prog, "seed {seed}");
        }
    }

    #[test]
    fn target_length_is_respected() {
        for seed in 0..50 {
            let short = Song::plan(&opts(seed, Some(130.0))).total_secs();
            let long = Song::plan(&opts(seed, Some(230.0))).total_secs();
            assert!(short < long, "seed {seed}: {short} vs {long}");
            assert!(short < 175.0, "seed {seed}: short {short}");
            assert!(long > 180.0, "seed {seed}: long {long}");
        }
    }

    #[test]
    fn voices_force_a_radio_intro_long_enough() {
        let mut o = opts(3, None);
        o.voice_secs = vec![6.0, 4.5];
        let s = Song::plan(&o);
        assert_eq!(s.radio, Some(Content::Voices));
        let intro_secs = s.secs[0].bars as f32 * s.bar_secs();
        assert!(intro_secs >= 2.2 + 6.0 + 4.5 + 1.4, "intro {intro_secs}");
    }
}
