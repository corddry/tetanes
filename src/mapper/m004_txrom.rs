//! `TxROM`/`MMC3` (Mapper 004)
//!
//! <https://wiki.nesdev.com/w/index.php/TxROM>
//! <https://wiki.nesdev.com/w/index.php/MMC3>

use crate::{
    cart::Cart,
    common::{Clock, Kind, Regional, Reset},
    mapper::{Mapped, MappedRead, MappedWrite, Mapper, MemMap},
    mem::MemBanks,
    ppu::Mirroring,
};
use serde::{Deserialize, Serialize};

// http://forums.nesdev.com/viewtopic.php?p=62546#p62546
// MMC3
// Conquest of the Crystal Palace (MMC3B S 9039 1 DB)
// Kickle Cubicle (MMC3B S 9031 3 DA)
// M.C. Kids (MMC3B S 9152 3 AB)
// Mega Man 3 (MMC3B S 9046 1 DB)
// Super Mario Bros. 3 (MMC3B S 9027 5 A)
// Startropics (MMC6B P 03'5)
// Batman (MMC3B 9006KP006)
// Golgo 13: The Mafat Conspiracy (MMC3B 9016KP051)
// Crystalis (MMC3B 9024KPO53)
// Legacy of the Wizard (MMC3A 8940EP)
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[must_use]
pub enum Mmc3Revision {
    /// MMC3 Revision A
    A,
    /// MMC3 Revisions B & C
    BC,
    /// Acclaims MMC3 clone - clocks on falling edge
    Acc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[must_use]
struct TxRegs {
    bank_select: u8,
    bank_values: [u8; 8],
    irq_latch: u8,
    irq_counter: u8,
    irq_enabled: bool,
    irq_reload: bool,
    last_clock: u16,
}

impl TxRegs {
    const fn new() -> Self {
        Self {
            bank_select: 0x00,
            bank_values: [0x00; 8],
            irq_latch: 0x00,
            irq_counter: 0x00,
            irq_enabled: false,
            irq_reload: false,
            last_clock: 0x0000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[must_use]
pub struct Txrom {
    regs: TxRegs,
    mirroring: Mirroring,
    irq_pending: bool,
    revision: Mmc3Revision,
    chr_banks: MemBanks,
    prg_ram_banks: MemBanks,
    prg_rom_banks: MemBanks,
}

impl Txrom {
    const PRG_WINDOW: usize = 8 * 1024;
    const CHR_WINDOW: usize = 1024;

    const FOUR_SCREEN_RAM_SIZE: usize = 4 * 1024;
    const PRG_RAM_SIZE: usize = 8 * 1024;
    const CHR_RAM_SIZE: usize = 8 * 1024;

    const PRG_MODE_MASK: u8 = 0x40; // Bit 6 of bank select
    const CHR_INVERSION_MASK: u8 = 0x80; // Bit 7 of bank select

    pub fn load(cart: &mut Cart) -> Mapper {
        cart.add_prg_ram(Self::PRG_RAM_SIZE);
        if cart.mirroring() == Mirroring::FourScreen {
            cart.add_ex_ram(Self::FOUR_SCREEN_RAM_SIZE);
        }
        if !cart.has_chr() {
            cart.add_chr_ram(Self::CHR_RAM_SIZE);
        };
        let mut txrom = Self {
            regs: TxRegs::new(),
            mirroring: cart.mirroring(),
            irq_pending: false,
            revision: Mmc3Revision::BC, // TODO compare to known games
            chr_banks: MemBanks::new(0x0000, 0x1FFF, cart.chr_len(), Self::CHR_WINDOW),
            prg_ram_banks: MemBanks::new(0x6000, 0x7FFF, cart.prg_ram.len(), Self::PRG_WINDOW),
            prg_rom_banks: MemBanks::new(0x8000, 0xFFFF, cart.prg_rom.len(), Self::PRG_WINDOW),
        };
        let last_bank = txrom.prg_rom_banks.last();
        txrom.prg_rom_banks.set(2, last_bank - 1);
        txrom.prg_rom_banks.set(3, last_bank);
        txrom.into()
    }

    #[inline]
    pub fn set_revision(&mut self, revision: Mmc3Revision) {
        self.revision = revision;
    }

    fn update_banks(&mut self) {
        let prg_last = self.prg_rom_banks.last();
        let prg_lo = self.regs.bank_values[6] as usize;
        let prg_hi = self.regs.bank_values[7] as usize;
        if self.regs.bank_select & Self::PRG_MODE_MASK == Self::PRG_MODE_MASK {
            self.prg_rom_banks.set(0, prg_last - 1);
            self.prg_rom_banks.set(1, prg_hi);
            self.prg_rom_banks.set(2, prg_lo);
        } else {
            self.prg_rom_banks.set(0, prg_lo);
            self.prg_rom_banks.set(1, prg_hi);
            self.prg_rom_banks.set(2, prg_last - 1);
        }
        self.prg_rom_banks.set(3, prg_last);

        // 1: two 2K banks at $1000-$1FFF, four 1 KB banks at $0000-$0FFF
        // 0: two 2K banks at $0000-$0FFF, four 1 KB banks at $1000-$1FFF
        let chr = self.regs.bank_values;
        if self.regs.bank_select & Self::CHR_INVERSION_MASK == Self::CHR_INVERSION_MASK {
            self.chr_banks.set(0, chr[2] as usize);
            self.chr_banks.set(1, chr[3] as usize);
            self.chr_banks.set(2, chr[4] as usize);
            self.chr_banks.set(3, chr[5] as usize);
            self.chr_banks.set_range(4, 5, (chr[0] & 0xFE) as usize);
            self.chr_banks.set_range(6, 7, (chr[1] & 0xFE) as usize);
        } else {
            self.chr_banks.set_range(0, 1, (chr[0] & 0xFE) as usize);
            self.chr_banks.set_range(2, 3, (chr[1] & 0xFE) as usize);
            self.chr_banks.set(4, chr[2] as usize);
            self.chr_banks.set(5, chr[3] as usize);
            self.chr_banks.set(6, chr[4] as usize);
            self.chr_banks.set(7, chr[5] as usize);
        }
    }

    fn clock_irq(&mut self, addr: u16) {
        if addr < 0x2000 {
            let next_clock = (addr >> 12) & 1;
            let (last, next) = if self.revision == Mmc3Revision::Acc {
                (1, 0)
            } else {
                (0, 1)
            };
            if self.regs.last_clock == last && next_clock == next {
                let counter = self.regs.irq_counter;
                if counter == 0 || self.regs.irq_reload {
                    self.regs.irq_counter = self.regs.irq_latch;
                } else {
                    self.regs.irq_counter -= 1;
                }
                if (counter & 0x01 == 0x01
                    || self.revision == Mmc3Revision::BC
                    || self.regs.irq_reload)
                    && self.regs.irq_counter == 0
                    && self.regs.irq_enabled
                {
                    self.irq_pending = true;
                }
                self.regs.irq_reload = false;
            }
            self.regs.last_clock = next_clock;
        }
    }
}

impl Mapped for Txrom {
    #[inline]
    fn irq_pending(&self) -> bool {
        self.irq_pending
    }

    #[inline]
    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    #[inline]
    fn set_mirroring(&mut self, mirroring: Mirroring) {
        self.mirroring = mirroring;
    }

    #[inline]
    fn ppu_bus_read(&mut self, addr: u16) {
        self.clock_irq(addr);
    }

    #[inline]
    fn ppu_bus_write(&mut self, addr: u16, _val: u8) {
        self.clock_irq(addr);
    }
}

impl MemMap for Txrom {
    // PPU $0000..=$07FF (or $1000..=$17FF) 2K CHR-ROM/RAM Bank 1 Switchable --+
    // PPU $0800..=$0FFF (or $1800..=$1FFF) 2K CHR-ROM/RAM Bank 2 Switchable --|-+
    // PPU $1000..=$13FF (or $0000..=$03FF) 1K CHR-ROM/RAM Bank 3 Switchable --+ |
    // PPU $1400..=$17FF (or $0400..=$07FF) 1K CHR-ROM/RAM Bank 4 Switchable --+ |
    // PPU $1800..=$1BFF (or $0800..=$0BFF) 1K CHR-ROM/RAM Bank 5 Switchable ----+
    // PPU $1C00..=$1FFF (or $0C00..=$0FFF) 1K CHR-ROM/RAM Bank 6 Switchable ----+
    // PPU $2000..=$3EFF FourScreen Mirroring (optional)

    // CPU $6000..=$7FFF 8K PRG-RAM Bank (optional)
    // CPU $8000..=$9FFF (or $C000..=$DFFF) 8K PRG-ROM Bank 1 Switchable
    // CPU $A000..=$BFFF 8K PRG-ROM Bank 2 Switchable
    // CPU $C000..=$DFFF (or $8000..=$9FFF) 8K PRG-ROM Bank 3 Fixed to second-to-last Bank
    // CPU $E000..=$FFFF 8K PRG-ROM Bank 4 Fixed to Last

    #[inline]
    fn map_read(&mut self, addr: u16) -> MappedRead {
        self.clock_irq(addr);
        self.map_peek(addr)
    }

    fn map_peek(&self, addr: u16) -> MappedRead {
        match addr {
            0x0000..=0x1FFF => MappedRead::Chr(self.chr_banks.translate(addr)),
            0x2000..=0x3EFF if self.mirroring == Mirroring::FourScreen => {
                MappedRead::ExRam((addr & 0x1FFF) as usize)
            }
            0x6000..=0x7FFF => MappedRead::PrgRam(self.prg_ram_banks.translate(addr)),
            0x8000..=0xFFFF => MappedRead::PrgRom(self.prg_rom_banks.translate(addr)),
            _ => MappedRead::None,
        }
    }

    fn map_write(&mut self, addr: u16, val: u8) -> MappedWrite {
        match addr {
            0x0000..=0x1FFF => MappedWrite::Chr(self.chr_banks.translate(addr), val),
            0x2000..=0x3EFF if self.mirroring == Mirroring::FourScreen => {
                MappedWrite::ExRam((addr & 0x1FFF) as usize, val)
            }
            0x6000..=0x7FFF => MappedWrite::PrgRam(self.prg_ram_banks.translate(addr), val),
            0x8000..=0xFFFF => {
                //  7654 3210
                // `CPMx xRRR`
                //  |||   +++- Specify which bank register to update on next write to Bank Data register
                //  |||        0: Select 2K CHR bank at PPU $0000-$07FF (or $1000-$17FF);
                //  |||        1: Select 2K CHR bank at PPU $0800-$0FFF (or $1800-$1FFF);
                //  |||        2: Select 1K CHR bank at PPU $1000-$13FF (or $0000-$03FF);
                //  |||        3: Select 1K CHR bank at PPU $1400-$17FF (or $0400-$07FF);
                //  |||        4: Select 1K CHR bank at PPU $1800-$1BFF (or $0800-$0BFF);
                //  |||        5: Select 1K CHR bank at PPU $1C00-$1FFF (or $0C00-$0FFF);
                //  |||        6: Select 8K PRG-ROM bank at $8000-$9FFF (or $C000-$DFFF);
                //  |||        7: Select 8K PRG-ROM bank at $A000-$BFFF
                //  ||+------- Nothing on the MMC3, see MMC6
                //  |+-------- PRG-ROM bank mode (0: $8000-$9FFF swappable,
                //  |                                $C000-$DFFF fixed to second-last bank;
                //  |                             1: $C000-$DFFF swappable,
                //  |                                $8000-$9FFF fixed to second-last bank)
                //  +--------- CHR A12 inversion (0: two 2K banks at $0000-$0FFF,
                //                                   four 1K banks at $1000-$1FFF;
                //                                1: two 2K banks at $1000-$1FFF,
                //                                   four 1K banks at $0000-$0FFF)
                //
                // Match only $8000/1, $A000/1, $C000/1, and $E000/1
                match addr & 0xE001 {
                    0x8000 => {
                        self.regs.bank_select = val;
                        self.update_banks();
                    }
                    0x8001 => {
                        let bank = self.regs.bank_select & 0x07;
                        self.regs.bank_values[bank as usize] = val;
                        self.update_banks();
                    }
                    0xA000 => {
                        if self.mirroring != Mirroring::FourScreen {
                            self.mirroring = match val & 0x01 {
                                0 => Mirroring::Vertical,
                                1 => Mirroring::Horizontal,
                                _ => unreachable!("impossible mirroring"),
                            };
                            self.update_banks();
                        }
                    }
                    0xA001 => {
                        // TODO RAM protect? Might conflict with MMC6
                    }
                    // IRQ
                    0xC000 => self.regs.irq_latch = val,
                    0xC001 => self.regs.irq_reload = true,
                    0xE000 => {
                        self.irq_pending = false;
                        self.regs.irq_enabled = false;
                    }
                    0xE001 => self.regs.irq_enabled = true,
                    _ => unreachable!("impossible address"),
                }
                MappedWrite::None
            }
            _ => MappedWrite::None,
        }
    }
}

impl Reset for Txrom {
    fn reset(&mut self, _kind: Kind) {
        self.irq_pending = false;
        self.regs = TxRegs::new();
        self.update_banks();
    }
}

impl Clock for Txrom {}
impl Regional for Txrom {}

#[cfg(test)]
mod tests {
    // use crate::test_roms;

    // test_roms!(
    //     "test_roms/mapper/m004_txrom",
    //     a12_clocking,
    //     clocking,
    //     details,
    //     rev_b,
    //     scanline_timing,
    //     big_chr_ram,
    //     rev_a,
    // );
}
