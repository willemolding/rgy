use crate::cgb::Cgb;
use crate::cpu::Cpu;
use crate::debug::Debugger;
use crate::device::Device;
use crate::dma::Dma;
use crate::fc::FreqControl;
use crate::gpu::Gpu;
use crate::hardware::{Hardware, HardwareHandle};
use crate::ic::Ic;
use crate::joypad::Joypad;
use crate::mbc::Mbc;
use crate::mmu::Mmu;
use crate::serial::Serial;
use crate::sound::Sound;
use crate::timer::Timer;
use log::*;

use alloc::vec::Vec;
use alloc::vec;

/// Configuration of the emulator.
pub struct Config {
    /// CPU frequency.
    pub(crate) freq: u64,
    /// Cycle sampling count in the CPU frequency controller.
    pub(crate) sample: u64,
    /// Delay unit in CPU frequency controller.
    pub(crate) delay_unit: u64,
    /// Don't adjust CPU frequency.
    pub(crate) native_speed: bool,
}

impl Config {
    /// Create the default configuration.
    pub fn new() -> Self {
        let freq = 4194300; // 4.1943 MHz
        Self {
            freq,
            sample: freq / 1000,
            delay_unit: 10,
            native_speed: false,
        }
    }

    /// Set the CPU frequency.
    pub fn freq(mut self, freq: u64) -> Self {
        self.freq = freq;
        self
    }

    /// Set the sampling count of the CPU frequency controller.
    pub fn sample(mut self, sample: u64) -> Self {
        self.sample = sample;
        self
    }

    /// Set the delay unit.
    pub fn delay_unit(mut self, delay: u64) -> Self {
        self.delay_unit = delay;
        self
    }

    /// Set the flag to run at native speed.
    pub fn native_speed(mut self, native: bool) -> Self {
        self.native_speed = native;
        self
    }
}

/// Represents the entire emulator context.
pub struct System<D> {
    cfg: Config,
    hw: HardwareHandle,
    fc: FreqControl,
    cpu: Cpu,
    mmu: Option<Mmu>,
    dbg: Device<D>,
    ic: Device<Ic>,
    gpu: Device<Gpu>,
    joypad: Device<Joypad>,
    timer: Device<Timer>,
    serial: Device<Serial>,
    dma: Device<Dma>,
}

impl<D> System<D>
where
    D: Debugger + 'static,
{
    /// Create a new emulator context.
    pub fn new<T>(cfg: Config, rom: &[u8], ram: Vec<u8>, hw: T, dbg: D) -> Self
    where
        T: Hardware + 'static,
    {
        info!("Initializing...");

        let hw = HardwareHandle::new(hw);

        let mut fc = FreqControl::new(hw.clone(), &cfg);

        let dbg = Device::mediate(dbg);
        let cpu = Cpu::new();
        let mut mmu = Mmu::new(ram);
        let sound = Device::new(Sound::new(hw.clone()));
        let ic = Device::new(Ic::new());
        let irq = ic.borrow().irq().clone();
        let gpu = Device::new(Gpu::new(hw.clone(), irq.clone()));
        let joypad = Device::new(Joypad::new(hw.clone(), irq.clone()));
        let timer = Device::new(Timer::new(irq.clone()));
        let serial = Device::new(Serial::new(hw.clone(), irq.clone()));
        let mbc = Device::new(Mbc::new(hw.clone(), rom.to_vec()));
        let cgb = Device::new(Cgb::new());
        let dma = Device::new(Dma::new());

        mmu.add_handler((0x0000, 0xffff), dbg.handler());

        mmu.add_handler((0xc000, 0xdfff), cgb.handler());
        mmu.add_handler((0xff4d, 0xff4d), cgb.handler());
        mmu.add_handler((0xff56, 0xff56), cgb.handler());
        mmu.add_handler((0xff70, 0xff70), cgb.handler());

        mmu.add_handler((0x0000, 0x7fff), mbc.handler());
        mmu.add_handler((0xff50, 0xff50), mbc.handler());
        mmu.add_handler((0xa000, 0xbfff), mbc.handler());
        mmu.add_handler((0xff10, 0xff3f), sound.handler());

        mmu.add_handler((0xff46, 0xff46), dma.handler());

        mmu.add_handler((0x8000, 0x9fff), gpu.handler());
        mmu.add_handler((0xff40, 0xff55), gpu.handler());
        mmu.add_handler((0xff68, 0xff6b), gpu.handler());

        mmu.add_handler((0xff0f, 0xff0f), ic.handler());
        mmu.add_handler((0xffff, 0xffff), ic.handler());
        mmu.add_handler((0xff00, 0xff00), joypad.handler());
        mmu.add_handler((0xff04, 0xff07), timer.handler());
        mmu.add_handler((0xff01, 0xff02), serial.handler());

        dbg.borrow_mut().init(&mmu);

        info!("Starting...");

        fc.reset();

        let mmu = Some(mmu);

        Self {
            cfg,
            hw,
            fc,
            cpu,
            mmu,
            dbg,
            ic,
            gpu,
            joypad,
            timer,
            serial,
            dma,
        }
    }

    fn step(&mut self, mut mmu: Mmu, gpu_enabled: bool) -> Mmu {
        {
            let mut dbg = self.dbg.borrow_mut();
            dbg.check_signal();
            dbg.take_cpu_snapshot(self.cpu.clone());
            dbg.on_decode(&mmu);
        }

        let mut time = self.cpu.execute(&mut mmu);

        time += self.cpu.check_interrupt(&mut mmu, &self.ic);

        self.dma.borrow_mut().step(&mut mmu);
        if gpu_enabled {
            self.gpu.borrow_mut().step(time, &mut mmu);
        }
        self.timer.borrow_mut().step(time);
        self.serial.borrow_mut().step(time);
        self.joypad.borrow_mut().poll();

        if !self.cfg.native_speed {
            self.fc.adjust(time);
        }

        mmu
    }

    /// Run a single step of emulation.
    /// This function needs to be called repeatedly until it returns `false`.
    /// Returning `false` indicates the end of emulation, and the functions shouldn't be called again.
    pub fn poll(&mut self, gpu_enabled: bool) -> bool {
        if !self.hw.get().borrow_mut().sched() {
            return false;
        }

        let mmu = self.mmu.take().unwrap();
        self.mmu = Some(self.step(mmu, gpu_enabled));

        true
    }

    /// Read a byte from the given address in the MMU
    pub fn mmu_get8(&self, addr: u16) -> u8 {
        self.mmu.as_ref().expect("memory not initialized").get8(addr)
    }

    /// Read a byte from the given address in the MMU
    pub fn mmu_get16(&self, addr: u16) -> u16 {
        self.mmu.as_ref().expect("memory not initialized").get16(addr)
    }

    /// dump the array backing the memory
    pub fn mmu_dump(&self) -> &[u8] {
        self.mmu.as_ref().expect("memory not initialized").dump()
    }
}

/// Run the emulator with the given configuration.
pub fn run<T: Hardware + 'static>(cfg: Config, rom: &[u8], hw: T) {
    run_inner(cfg, rom, hw, Debugger::empty())
}

/// Run the emulator with the given configuration and debugger.
pub fn run_debug<T: Hardware + 'static, D: Debugger + 'static>(
    cfg: Config,
    rom: &[u8],
    hw: T,
    dbg: D,
) {
    run_inner(cfg, rom, hw, dbg)
}

fn run_inner<T: Hardware + 'static, D: Debugger + 'static>(cfg: Config, rom: &[u8], hw: T, dbg: D) {
    let mut sys = System::new(cfg, rom, vec![0u8; 0x10000], hw, dbg);
    while sys.poll(true) {}
}
