use std::rc::Rc;
use std::cell::RefCell;

use crate::mmu::{MemHandler, MemRead, MemWrite, Mmu};

pub type Stream = FnMut(f32) -> Option<f32> + Send + Sync + 'static;

pub trait Speaker {
    fn play(&self, stream: Box<Stream>);

    fn stop(&self);
}

pub struct Sound {
    inner: Rc<RefCell<Inner>>,
}

impl Sound {
    pub fn new(speaker: Box<Speaker>) -> Sound {
        Sound {
            inner: Rc::new(RefCell::new(Inner::new(speaker))),
        }
    }

    pub fn handler(&self) -> SoundMemHandler {
        SoundMemHandler::new(self.inner.clone())
    }
}

#[derive(Debug)]
enum WaveDuty {
    P125,
    P250,
    P500,
    P750,
}

impl From<WaveDuty> for u8 {
    fn from(s: WaveDuty) -> u8 {
        match s {
            WaveDuty::P125 => 0,
            WaveDuty::P250 => 1,
            WaveDuty::P500 => 2,
            WaveDuty::P750 => 3,
        }
    }
}

impl From<u8> for WaveDuty {
    fn from(s: u8) -> WaveDuty {
        match s {
            0 => WaveDuty::P125,
            1 => WaveDuty::P250,
            2 => WaveDuty::P500,
            3 => WaveDuty::P750,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug)]
struct Tone {
    sweep_time: usize,
    sweep_sub: bool,
    sweep_shift: usize,
    sound_len: usize,
    wave_duty: WaveDuty,
    env_init: usize,
    env_inc: bool,
    env_count: usize,
    counter: bool,
    freq: usize,
}

struct Inner {
    speaker: Box<Speaker>,
    tone: Tone,
}

impl Inner {
    fn new(speaker: Box<Speaker>) -> Inner {
        let tone = Tone {
            sweep_time: 0,
            sweep_sub: false,
            sweep_shift: 0,
            sound_len: 0,
            wave_duty: WaveDuty::P125,
            env_init: 0,
            env_inc: false,
            env_count: 0,
            counter: false,
            freq: 0,
        };

        Inner { speaker, tone }
    }

    fn on_read(&mut self, mmu: &Mmu, addr: u16) -> MemRead {
        MemRead::PassThrough
    }

    fn on_write(&mut self, mmu: &Mmu, addr: u16, value: u8) -> MemWrite {
        if addr == 0xff10 {
            self.tone.sweep_time = ((value >> 4) & 0x7) as usize;
            self.tone.sweep_sub = value & 0x08 != 0;
            self.tone.sweep_shift = (value & 0x07) as usize;
        } else if addr == 0xff11 {
            self.tone.wave_duty = (value >> 6).into();
            self.tone.sound_len = (value & 0x3f) as usize;
        } else if addr == 0xff12 {
            self.tone.env_init = (value >> 4) as usize;
            self.tone.env_inc = value & 0x08 != 0;
            self.tone.env_count = (value & 0x7) as usize;
        } else if addr == 0xff13 {
            self.tone.freq = (self.tone.freq & !0xff) | value as usize;
        } else if addr == 0xff14 {
            self.tone.counter = value & 0x40 != 0;
            self.tone.freq = (self.tone.freq & !0x700) | (((value & 0x7) as usize) << 8);
            if value & 0x80 != 0 {
                debug!("Play: {:#?}", self.tone);
                self.play_tone1();
            }
        }

        MemWrite::Block
    }

    fn play_tone1(&mut self) {
        let vol = self.tone.env_init as f32 / 15.0;
        let env_count = self.tone.env_count as f32;
        let diff = vol / 15.0 as f32;
        let diff = if self.tone.env_inc { diff } else { diff * -1.0 };
        let freq = 131072f32 / (2048f32 - self.tone.freq as f32);

        debug!("Freq: {}", freq);

        let mut clock = 0f32;
        let mut env_clock = 0f32;
        let mut vol = vol;

        self.speaker.play(Box::new(move |rate| {
            // Envelop
            env_clock += 1.0;
            if env_clock >= rate * env_count / 64.0 {
                env_clock = 0.0;
                vol += diff;
                vol = if vol < 0.0 {
                    0.0
                } else if vol > 1.0 {
                    1.0
                } else {
                    vol
                };
            }

            // Sign wave
            clock += 1.0;
            Some(((clock % rate) * freq * 2.0 * 3.141592 / rate).sin() * vol)
        }));
    }

    fn play_tone2(&mut self) {}

    fn play_wave(&mut self) {}

    fn play_noise(&mut self) {}
}

pub struct SoundMemHandler {
    inner: Rc<RefCell<Inner>>,
}

impl SoundMemHandler {
    fn new(inner: Rc<RefCell<Inner>>) -> SoundMemHandler {
        SoundMemHandler { inner }
    }
}

impl MemHandler for SoundMemHandler {
    fn on_read(&self, mmu: &Mmu, addr: u16) -> MemRead {
        self.inner.borrow_mut().on_read(mmu, addr)
    }

    fn on_write(&self, mmu: &Mmu, addr: u16, value: u8) -> MemWrite {
        self.inner.borrow_mut().on_write(mmu, addr, value)
    }
}