//! Low level unrestricted access to the CC1101 radio chip.
use hal::blocking::spi::{Transfer, Write, WriteIter};
use hal::digital::v2::OutputPin;
use heapless::Vec;
#[macro_use]
mod macros;
mod access;
mod traits;

pub mod convert;
pub mod registers;
pub mod types;

use self::registers::*;

pub const FXOSC: u64 = 26_000_000;

pub struct Cc1101<SPI, CS, GDO2> {
    pub(crate) spi: SPI,
    pub(crate) cs: CS,
    // gdo0: GDO0,
    pub(crate) gdo2: GDO2,
}

#[derive(Debug)]
pub enum Error<SpiE, GpioE> {
    Spi(SpiE),
    Gpio(GpioE),
}

impl<SPI, CS, GDO2, SpiE, GpioE> Cc1101<SPI, CS, GDO2>
where
    SPI: Transfer<u8, Error = SpiE> + Write<u8, Error = SpiE>,
    CS: OutputPin<Error = GpioE>,
{
    pub fn new(spi: SPI, cs: CS, gdo2: GDO2) -> Result<Self, Error<SpiE, GpioE>> {
        let cc1101 = Cc1101 {
            spi,
            cs,
            gdo2,
        };

        Ok(cc1101)
    }

    pub fn read_register<R>(&mut self, reg: R) -> Result<u8, Error<SpiE, GpioE>>
    where
        R: Into<Register>,
    {
        self.cs.set_low().map_err(Error::Gpio)?;
        let mut buffer = [reg.into().raddr(), 0u8];
        self.spi.transfer(&mut buffer).map_err(Error::Spi)?;
        self.cs.set_high().map_err(Error::Gpio)?;
        Ok(buffer[1])
    }

    pub fn read_fifo(
        &mut self,
        addr: &mut u8,
        len: &mut u8,
        buf: &mut [u8],
    ) -> Result<(), Error<SpiE, GpioE>> {
        let mut buffer = [Command::FIFO.addr() | 0xC0, 0, 0];

        self.cs.set_low().map_err(Error::Gpio)?;
        self.spi.transfer(&mut buffer).map_err(Error::Spi)?;
        self.spi.transfer(buf).map_err(Error::Spi)?;
        self.cs.set_high().map_err(Error::Gpio)?;

        *len = buffer[1];
        *addr = buffer[2];

        Ok(())
    }

    pub fn write_strobe(&mut self, com: Command) -> Result<(), Error<SpiE, GpioE>> {
        self.cs.set_low().map_err(Error::Gpio)?;
        self.spi.write(&[com.addr()]).map_err(Error::Spi)?;
        self.cs.set_high().map_err(Error::Gpio)?;
        Ok(())
    }

    pub fn write_register<R>(&mut self, reg: R, byte: u8) -> Result<(), Error<SpiE, GpioE>>
    where
        R: Into<Register>,
    {
        self.cs.set_low().map_err(Error::Gpio)?;
        self.spi.write(&mut [reg.into().waddr(), byte]).map_err(Error::Spi)?;
        self.cs.set_high().map_err(Error::Gpio)?;
        Ok(())
    }

    pub fn write_burst<R>(&mut self, reg: R, bytes: &mut [u8]) -> Result<(), Error<SpiE, GpioE>>
    where
        R: Into<Register>,
    {
        // let mut payload: Vec<u8, 64> = Vec::new();
        // payload.push(reg.into().waddr() | Command::BURSTFLG.addr());
        // payload.extend(bytes.iter().cloned());

        let mut payload_u8: [u8; 64] = [0; 64];
        payload_u8[0] = reg.into().waddr() | Command::BURSTFLG.addr();
        payload_u8[1..bytes.len() + 1].copy_from_slice(&bytes);
        self.cs.set_low().map_err(Error::Gpio)?;

        self.spi.write(&mut payload_u8[..bytes.len()]).map_err(Error::Spi)?;
        self.cs.set_high().map_err(Error::Gpio)?;
        Ok(())
    }
    pub fn modify_register<R, F>(&mut self, reg: R, f: F) -> Result<(), Error<SpiE, GpioE>>
    where
        R: Into<Register> + Copy,
        F: FnOnce(u8) -> u8,
    {
        let r = self.read_register(reg)?;
        self.write_register(reg, f(r))?;
        Ok(())
    }
}
