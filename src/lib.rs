#![no_std]

extern crate embedded_hal as hal;

use hal::blocking::spi::{Transfer, Write};
use hal::digital::v2::{InputPin, OutputPin};

#[macro_use]
pub mod lowlevel;
mod rssi;

use lowlevel::convert::*;
use lowlevel::registers::*;
use lowlevel::types::*;
use rssi::rssi_to_dbm;
const MAX_TX: usize = 256;
/// CC1101 errors.
#[derive(Debug)]
pub enum Error<SpiE, GpioE> {
    /// The RX FIFO buffer overflowed, too small buffer for configured packet length.
    RxOverflow,
    /// Corrupt packet received with invalid CRC.
    CrcMismatch,
    /// Platform-dependent SPI-errors, such as IO errors.
    Spi(SpiE),
    /// Platform-dependent GPIO-errors, such as IO errors.
    Gpio(GpioE),
}

impl<SpiE, GpioE> From<lowlevel::Error<SpiE, GpioE>> for Error<SpiE, GpioE> {
    fn from(e: lowlevel::Error<SpiE, GpioE>) -> Self {
        match e {
            lowlevel::Error::Spi(inner) => Error::Spi(inner),
            lowlevel::Error::Gpio(inner) => Error::Gpio(inner),
        }
    }
}

/// High level API for interacting with the CC1101 radio chip.
pub struct Cc1101<SPI, CS, GDO2>(lowlevel::Cc1101<SPI, CS, GDO2>);

impl<SPI, CS, GDO2, SpiE, GpioE> Cc1101<SPI, CS, GDO2>
where
    SPI: Transfer<u8, Error = SpiE> + Write<u8, Error = SpiE>,
    CS: OutputPin<Error = GpioE>,
    GDO2: InputPin<Error = GpioE>,
{
    pub fn new(spi: SPI, cs: CS, gdo2: GDO2) -> Result<Self, Error<SpiE, GpioE>> {
        Ok(Cc1101(lowlevel::Cc1101::new(spi, cs, gdo2)?))
    }

    pub fn set_frequency(&mut self, hz: u64) -> Result<(), Error<SpiE, GpioE>> {
        let (freq0, freq1, freq2) = from_frequency(hz);
        self.0.write_register(Config::FREQ0, freq0)?;
        self.0.write_register(Config::FREQ1, freq1)?;
        self.0.write_register(Config::FREQ2, freq2)?;
        Ok(())
    }

    pub fn set_deviation(&mut self, deviation: u64) -> Result<(), Error<SpiE, GpioE>> {
        let (mantissa, exponent) = from_deviation(deviation);
        self.0.write_register(
            Config::DEVIATN,
            DEVIATN::default().deviation_m(mantissa).deviation_e(exponent).bits(),
        )?;
        Ok(())
    }

    pub fn set_data_rate(&mut self, baud: u64) -> Result<(), Error<SpiE, GpioE>> {
        let (mantissa, exponent) = from_drate(baud);
        self.0
            .modify_register(Config::MDMCFG4, |r| MDMCFG4(r).modify().drate_e(exponent).bits())?;
        self.0.write_register(Config::MDMCFG3, MDMCFG3::default().drate_m(mantissa).bits())?;
        Ok(())
    }

    pub fn set_chanbw(&mut self, bandwidth: u64) -> Result<(), Error<SpiE, GpioE>> {
        let (mantissa, exponent) = from_chanbw(bandwidth);
        self.0.modify_register(Config::MDMCFG4, |r| {
            MDMCFG4(r).modify().chanbw_m(mantissa).chanbw_e(exponent).bits()
        })?;
        Ok(())
    }

    pub fn get_hw_info(&mut self) -> Result<(u8, u8), Error<SpiE, GpioE>> {
        let partnum = self.0.read_register(Status::PARTNUM)?;
        let version = self.0.read_register(Status::VERSION)?;
        Ok((partnum, version))
    }

    /// Received Signal Strength Indicator is an estimate of the signal power level in the chosen channel.
    pub fn get_rssi_dbm(&mut self) -> Result<i16, Error<SpiE, GpioE>> {
        Ok(rssi_to_dbm(self.0.read_register(Status::RSSI)?))
    }

    /// The Link Quality Indicator metric of the current quality of the received signal.
    pub fn get_lqi(&mut self) -> Result<u8, Error<SpiE, GpioE>> {
        let lqi = self.0.read_register(Status::LQI)?;
        Ok(lqi & !(1u8 << 7))
    }

    /// Configure the sync word to use, and at what level it should be verified.
    pub fn set_sync_mode(&mut self, sync_mode: SyncMode) -> Result<(), Error<SpiE, GpioE>> {
        let reset: u16 = (SYNC1::default().bits() as u16) << 8 | (SYNC0::default().bits() as u16);

        let (mode, word) = match sync_mode {
            SyncMode::Disabled => (SyncCheck::DISABLED, reset),
            SyncMode::MatchPartial(word) => (SyncCheck::CHECK_15_16, word),
            SyncMode::MatchPartialRepeated(word) => (SyncCheck::CHECK_30_32, word),
            SyncMode::MatchFull(word) => (SyncCheck::CHECK_16_16, word),
        };
        self.0.modify_register(Config::MDMCFG2, |r| {
            MDMCFG2(r).modify().sync_mode(mode.value()).bits()
        })?;
        self.0.write_register(Config::SYNC1, ((word >> 8) & 0xff) as u8)?;
        self.0.write_register(Config::SYNC0, (word & 0xff) as u8)?;
        Ok(())
    }

    /// Configure signal modulation.
    pub fn set_modulation(&mut self, format: Modulation) -> Result<(), Error<SpiE, GpioE>> {
        use lowlevel::types::ModFormat as MF;

        let value = match format {
            Modulation::BinaryFrequencyShiftKeying => MF::MOD_2FSK,
            Modulation::GaussianFrequencyShiftKeying => MF::MOD_GFSK,
            Modulation::OnOffKeying => MF::MOD_ASK_OOK,
            Modulation::FourFrequencyShiftKeying => MF::MOD_4FSK,
            Modulation::MinimumShiftKeying => MF::MOD_MSK,
        };
        self.0.modify_register(Config::MDMCFG2, |r| {
            MDMCFG2(r).modify().mod_format(value.value()).bits()
        })?;
        Ok(())
    }

    /// Configure device address, and address filtering.
    pub fn set_address_filter(&mut self, filter: AddressFilter) -> Result<(), Error<SpiE, GpioE>> {
        use lowlevel::types::AddressCheck as AC;

        let (mode, addr) = match filter {
            AddressFilter::Disabled => (AC::DISABLED, ADDR::default().bits()),
            AddressFilter::Device(addr) => (AC::SELF, addr),
            AddressFilter::DeviceLowBroadcast(addr) => (AC::SELF_LOW_BROADCAST, addr),
            AddressFilter::DeviceHighLowBroadcast(addr) => (AC::SELF_HIGH_LOW_BROADCAST, addr),
        };
        self.0.modify_register(Config::PKTCTRL1, |r| {
            PKTCTRL1(r).modify().adr_chk(mode.value()).bits()
        })?;
        self.0.write_register(Config::ADDR, addr)?;
        Ok(())
    }

    /// Configure packet mode, and length.
    pub fn set_packet_length(&mut self, length: PacketLength) -> Result<(), Error<SpiE, GpioE>> {
        use lowlevel::types::LengthConfig as LC;

        let (format, pktlen) = match length {
            PacketLength::Fixed(limit) => (LC::FIXED, limit),
            PacketLength::Variable(max_limit) => (LC::VARIABLE, max_limit),
            PacketLength::Infinite => (LC::INFINITE, PKTLEN::default().bits()),
        };
        self.0.modify_register(Config::PKTCTRL0, |r| {
            PKTCTRL0(r).modify().length_config(format.value()).bits()
        })?;
        self.0.write_register(Config::PKTLEN, pktlen)?;
        Ok(())
    }

    /// Set radio in Receive/Transmit/Idle mode.
    pub fn set_radio_mode(&mut self, radio_mode: RadioMode) -> Result<(), Error<SpiE, GpioE>> {
        let target = match radio_mode {
            RadioMode::Receive => {
                self.set_radio_mode(RadioMode::Idle)?;
                self.0.write_strobe(Command::SRX)?;
                MachineState::RX
            }
            RadioMode::Transmit => {
                self.set_radio_mode(RadioMode::Idle)?;
                self.0.write_strobe(Command::STX)?;
                MachineState::TX
            }
            RadioMode::Idle => {
                self.0.write_strobe(Command::SIDLE)?;
                MachineState::IDLE
            }
        };
        self.await_machine_state(target)
    }

    /// Configure some default settings, to be removed in the future.
    #[cfg_attr(rustfmt, rustfmt_skip)]
    pub fn set_defaults(&mut self) -> Result<(), Error<SpiE, GpioE>> {
        self.0.write_strobe(Command::SRES)?;

        self.0.write_register(Config::PKTCTRL0, PKTCTRL0::default()
            .white_data(0).bits()
        )?;

        self.0.write_register(Config::FSCTRL1, FSCTRL1::default()
            .freq_if(0x08).bits() // f_if = (f_osc / 2^10) * FREQ_IF
        )?;

        self.0.write_register(Config::MDMCFG2, MDMCFG2::default()
            .dem_dcfilt_off(1).bits()
        )?;

        self.0.write_register(Config::MCSM0, MCSM0::default()
            .fs_autocal(AutoCalibration::FROM_IDLE.value()).bits()
        )?;

        self.0.write_register(Config::AGCCTRL2, AGCCTRL2::default()
            .max_lna_gain(0x04).bits()
        )?;

        Ok(())
    }

    fn await_machine_state(&mut self, target: MachineState) -> Result<(), Error<SpiE, GpioE>> {
        loop {
            let marcstate = MARCSTATE(self.0.read_register(Status::MARCSTATE)?);
            if target.value() == marcstate.marc_state() {
                break;
            }
        }
        Ok(())
    }

    fn rx_bytes_available(&mut self) -> Result<u8, Error<SpiE, GpioE>> {
        let mut last = 0;

        loop {
            let rxbytes = RXBYTES(self.0.read_register(Status::RXBYTES)?);
            if rxbytes.rxfifo_overflow() == 1 {
                return Err(Error::RxOverflow);
            }

            let nbytes = rxbytes.num_rxbytes();
            if nbytes > 0 && nbytes == last {
                break;
            }

            last = nbytes;
        }
        Ok(last)
    }

    // Should also be able to configure MCSM1.RXOFF_MODE to declare what state
    // to enter after fully receiving a packet.
    // Possible targets: IDLE, FSTON, TX, RX
    pub fn receive(&mut self, addr: &mut u8, buf: &mut [u8]) -> Result<u8, Error<SpiE, GpioE>> {
        match self.rx_bytes_available() {
            Ok(_nbytes) => {
                let mut length = 0u8;
                self.0.read_fifo(addr, &mut length, buf)?;
                let lqi = self.0.read_register(Status::LQI)?;
                self.await_machine_state(MachineState::IDLE)?;
                self.0.write_strobe(Command::SFRX)?;
                if (lqi >> 7) != 1 {
                    Err(Error::CrcMismatch)
                } else {
                    Ok(length)
                }
            }
            Err(err) => {
                self.0.write_strobe(Command::SFRX)?;
                Err(err)
            }
        }
    }

    pub fn transmit(&mut self, payload: &[u8], len: u8) -> Result<(), Error<SpiE, GpioE>> {
        // let ret: u8 = PAYLOAD_TRANSMITTED;

        if len > 0 && len < 62 {
            self.0.write_register(Config::IOCFG0, 0x09)?;
            //
            let mut tx_buffer: [u8; 64] = [0; 64];
            tx_buffer[0] = len;
            tx_buffer[1..].copy_from_slice(payload);
            // // memcpy(tx_buffer + 1, payload, len);
            // // cc1101_idle_mode();
            self.set_radio_mode(RadioMode::Idle)?;
            // // cc1101_write_strobe(SFTX); // Flush TX_FIFO
            self.0.write_strobe(Command::SFTX)?;
            // self.set_radio_mode(RadioMode::Idle)?;
            // funcptr.delay_us(100); /TODO
            // cc1101_receive_mode();
            self.set_radio_mode(RadioMode::Receive)?;
            self.0.write_burst(Command::FIFO, &mut tx_buffer)?;
            // funcptr.delay_ms(1); // Wait for CCA to be asserted //TODO

            // for i in 0..100_000_000 {}
            // if (funcptr.gdo0()) { //TODO
            // Listen before Talk
            self.0.write_register(Config::IOCFG0, 0x06)?; //TODO ???
                                                          // cc1101_write_register(IOCFG0, 0x06);
                                                          // self.0.write_strobe(Command::STX)?; // Sends Data

            self.set_radio_mode(RadioMode::Transmit)?;
            // // Wait for GDO2 to be set -> sync transmitted
            let mut waiting_for_sync = true;
            while waiting_for_sync {
                if let Ok(gdo2_state) = self.0.gdo2.is_low() {
                    waiting_for_sync = gdo2_state;
                }
            }
            let mut waiting_for_transmit = true;
            while waiting_for_transmit {
                if let Ok(gdo2_state) = self.0.gdo2.is_low() {
                    waiting_for_transmit = !gdo2_state;
                }
            }
            self.set_radio_mode(RadioMode::Idle)?;
            // while (!funcptr.gdo2())
            // 	;
            //
            // // Wait for GDO2 to be cleared -> end of packet
            // while (funcptr.gdo2())
            // 	;
            // } else { //TODO
            //     cc1101_idle_mode();
            //     cc1101_write_strobe(SFTX); // Flush TX_FIFO
            //     funcptr.delay_us(100);
            //     // ret = NOISE_ON_CHANNEL;
            // }
        } else {
            // ret = PAYLOAD_LEN_OUT_OF_RANGE;
        }

        Ok(())
        // return ret;
    }
}

/// Modulation format configuration.
pub enum Modulation {
    /// 2-FSK.
    BinaryFrequencyShiftKeying,
    /// GFSK.
    GaussianFrequencyShiftKeying,
    /// ASK / OOK.
    OnOffKeying,
    /// 4-FSK.
    FourFrequencyShiftKeying,
    /// MSK.
    MinimumShiftKeying,
}

/// Packet length configuration.
pub enum PacketLength {
    /// Set packet length to a fixed value.
    Fixed(u8),
    /// Set upper bound of variable packet length.
    Variable(u8),
    /// Infinite packet length, streaming mode.
    Infinite,
}

/// Address check configuration.
pub enum AddressFilter {
    /// No address check.
    Disabled,
    /// Address check, no broadcast.
    Device(u8),
    /// Address check and 0 (0x00) broadcast.
    DeviceLowBroadcast(u8),
    /// Address check and 0 (0x00) and 255 (0xFF) broadcast.
    DeviceHighLowBroadcast(u8),
}

/// Radio operational mode.
pub enum RadioMode {
    Receive,
    Transmit,
    Idle,
}

/// Sync word configuration.
pub enum SyncMode {
    /// No sync word.
    Disabled,
    /// Match 15 of 16 bits of given sync word.
    MatchPartial(u16),
    /// Match 30 of 32 bits of a repetition of given sync word.
    MatchPartialRepeated(u16),
    /// Match 16 of 16 bits of given sync word.
    MatchFull(u16),
}
