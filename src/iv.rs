use embassy_stm32::interrupt;

use embassy_stm32::interrupt::InterruptExt;

use embassy_stm32::pac;

use embassy_stm32::peripherals::SUBGHZSPI;
use embassy_stm32::spi;
use embassy_stm32::Peripheral;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use embassy_sync::signal::Signal;
use embedded_hal::digital::OutputPin;
use embedded_hal::spi::Operation;
use embedded_hal_async::delay::DelayUs;
use lora_phy::mod_params::RadioError;
use lora_phy::mod_traits::InterfaceVariant;

/// Interrupt handler.

pub struct InterruptHandler {}

impl interrupt::typelevel::Handler<interrupt::typelevel::SUBGHZ_RADIO> for InterruptHandler {
    unsafe fn on_interrupt() {
        interrupt::SUBGHZ_RADIO.disable();
        IRQ_SIGNAL.signal(());
    }
}

static IRQ_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

pub struct SubGhzSpiDevice<'d, Tx, Rx>
where
    Tx: spi::TxDma<SUBGHZSPI>,
    Rx: spi::RxDma<SUBGHZSPI>,
{
    bus: spi::Spi<'d, SUBGHZSPI, Tx, Rx>,
}

impl<'d, Tx, Rx> SubGhzSpiDevice<'d, Tx, Rx>
where
    Tx: spi::TxDma<SUBGHZSPI>,
    Rx: spi::RxDma<SUBGHZSPI>,
{
    pub fn new(
        spi: impl Peripheral<P = SUBGHZSPI> + 'd,
        txdma: impl Peripheral<P = Tx> + 'd,
        rxdma: impl Peripheral<P = Rx> + 'd,
    ) -> Self {
        let bus = spi::Spi::new_subghz(spi, txdma, rxdma);
        Self { bus }
    }
}
/// Error returned by SPI device implementations in this crate.
#[derive(Eq, PartialEq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum SpiDeviceError {
    /// An operation on the inner SPI bus failed.
    Spi(spi::Error),
    /// DelayUs operations are not supported when the `time` Cargo feature is not enabled.
    DelayUsNotSupported,
}

impl embedded_hal_async::spi::Error for SpiDeviceError {
    fn kind(&self) -> embedded_hal_async::spi::ErrorKind {
        match self {
            Self::Spi(e) => e.kind(),
            Self::DelayUsNotSupported => embedded_hal_async::spi::ErrorKind::Other,
        }
    }
}

impl<Tx, Rx> embedded_hal_async::spi::ErrorType for SubGhzSpiDevice<'_, Tx, Rx>
where
    Tx: spi::TxDma<SUBGHZSPI>,
    Rx: spi::RxDma<SUBGHZSPI>,
{
    type Error = SpiDeviceError;
}

impl<Tx, Rx> embedded_hal_async::spi::SpiDevice for SubGhzSpiDevice<'_, Tx, Rx>
where
    Tx: spi::TxDma<SUBGHZSPI>,
    Rx: spi::RxDma<SUBGHZSPI>,
{
    async fn transaction(
        &mut self,
        operations: &mut [Operation<'_, u8>],
    ) -> Result<(), Self::Error> {
        pac::PWR.subghzspicr().modify(|w| w.set_nss(false));

        let op_res: Result<(), Self::Error> = try {
            for op in operations {
                match op {
                    Operation::Read(buf) => {
                        self.bus.read(buf).await.map_err(SpiDeviceError::Spi)?
                    }
                    Operation::Write(buf) => {
                        self.bus.write(buf).await.map_err(SpiDeviceError::Spi)?
                    }
                    Operation::Transfer(read, write) => self
                        .bus
                        .transfer(read, write)
                        .await
                        .map_err(SpiDeviceError::Spi)?,
                    Operation::TransferInPlace(buf) => self
                        .bus
                        .transfer_in_place(buf)
                        .await
                        .map_err(SpiDeviceError::Spi)?,
                    Operation::DelayUs(us) => embassy_time::Timer::after_micros(*us as _).await,
                }
            }
        };

        pac::PWR.subghzspicr().modify(|w| w.set_nss(true));

        op_res
    }
}

/// Base for the InterfaceVariant implementation for an stm32wl/sx1262 combination
pub struct Stm32wlInterfaceVariant<CTRL> {
    rf_switch_rx: Option<CTRL>,
    rf_switch_tx: Option<CTRL>,
}

impl<CTRL> Stm32wlInterfaceVariant<CTRL>
where
    CTRL: OutputPin,
{
    /// Create an InterfaceVariant instance for an stm32wl/sx1262 combination
    pub fn new(
        _irq: impl interrupt::typelevel::Binding<interrupt::typelevel::SUBGHZ_RADIO, InterruptHandler>,
        rf_switch_rx: Option<CTRL>,
        rf_switch_tx: Option<CTRL>,
    ) -> Result<Self, RadioError> {
        interrupt::SUBGHZ_RADIO.disable();
        Ok(Self {
            rf_switch_rx,
            rf_switch_tx,
        })
    }
}

impl<CTRL> InterfaceVariant for Stm32wlInterfaceVariant<CTRL>
where
    CTRL: OutputPin,
{
    async fn reset(&mut self, _delay: &mut impl DelayUs) -> Result<(), RadioError> {
        pac::RCC.csr().modify(|w| w.set_rfrst(true));
        pac::RCC.csr().modify(|w| w.set_rfrst(false));
        Ok(())
    }
    async fn wait_on_busy(&mut self) -> Result<(), RadioError> {
        while pac::PWR.sr2().read().rfbusys() {}
        Ok(())
    }

    async fn await_irq(&mut self) -> Result<(), RadioError> {
        unsafe { interrupt::SUBGHZ_RADIO.enable() };
        IRQ_SIGNAL.wait().await;
        Ok(())
    }

    async fn enable_rf_switch_rx(&mut self) -> Result<(), RadioError> {
        match &mut self.rf_switch_tx {
            Some(pin) => pin.set_low().map_err(|_| RadioError::RfSwitchTx)?,
            None => (),
        };
        match &mut self.rf_switch_rx {
            Some(pin) => pin.set_high().map_err(|_| RadioError::RfSwitchRx),
            None => Ok(()),
        }
    }
    async fn enable_rf_switch_tx(&mut self) -> Result<(), RadioError> {
        match &mut self.rf_switch_rx {
            Some(pin) => pin.set_low().map_err(|_| RadioError::RfSwitchRx)?,
            None => (),
        };
        match &mut self.rf_switch_tx {
            Some(pin) => pin.set_high().map_err(|_| RadioError::RfSwitchTx),
            None => Ok(()),
        }
    }
    async fn disable_rf_switch(&mut self) -> Result<(), RadioError> {
        match &mut self.rf_switch_rx {
            Some(pin) => pin.set_low().map_err(|_| RadioError::RfSwitchRx)?,
            None => (),
        };
        match &mut self.rf_switch_tx {
            Some(pin) => pin.set_low().map_err(|_| RadioError::RfSwitchTx),
            None => Ok(()),
        }
    }
}
