use embassy_stm32::interrupt;

use embassy_stm32::interrupt::InterruptExt;

use embassy_stm32::pac;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use embassy_sync::signal::Signal;
use embedded_hal::digital::OutputPin;
use embedded_hal_async::delay::DelayUs;
use lora_phy::mod_params::RadioError::*;
use lora_phy::mod_params::{BoardType, RadioError};
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

/// Base for the InterfaceVariant implementation for an stm32wl/sx1262 combination
pub struct Stm32wlInterfaceVariant<CTRL> {
    board_type: BoardType,
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
            board_type: BoardType::Stm32wlSx1262, // updated when associated with a specific LoRa board
            rf_switch_rx,
            rf_switch_tx,
        })
    }
}

impl<CTRL> InterfaceVariant for Stm32wlInterfaceVariant<CTRL>
where
    CTRL: OutputPin,
{
    fn set_board_type(&mut self, board_type: BoardType) {
        self.board_type = board_type;
    }
    async fn set_nss_low(&mut self) -> Result<(), RadioError> {
        let pwr = pac::PWR;
        pwr.subghzspicr().modify(|w| w.set_nss(false));
        Ok(())
    }
    async fn set_nss_high(&mut self) -> Result<(), RadioError> {
        let pwr = pac::PWR;
        pwr.subghzspicr().modify(|w| w.set_nss(true));
        Ok(())
    }
    async fn reset(&mut self, _delay: &mut impl DelayUs) -> Result<(), RadioError> {
        let rcc = pac::RCC;
        rcc.csr().modify(|w| w.set_rfrst(true));
        rcc.csr().modify(|w| w.set_rfrst(false));
        Ok(())
    }
    async fn wait_on_busy(&mut self) -> Result<(), RadioError> {
        let pwr = pac::PWR;
        while pwr.sr2().read().rfbusys() {}
        Ok(())
    }

    async fn await_irq(&mut self) -> Result<(), RadioError> {
        unsafe { interrupt::SUBGHZ_RADIO.enable() };
        IRQ_SIGNAL.wait().await;
        Ok(())
    }

    async fn enable_rf_switch_rx(&mut self) -> Result<(), RadioError> {
        match &mut self.rf_switch_tx {
            Some(pin) => pin.set_low().map_err(|_| RfSwitchTx)?,
            None => (),
        };
        match &mut self.rf_switch_rx {
            Some(pin) => pin.set_high().map_err(|_| RfSwitchRx),
            None => Ok(()),
        }
    }
    async fn enable_rf_switch_tx(&mut self) -> Result<(), RadioError> {
        match &mut self.rf_switch_rx {
            Some(pin) => pin.set_low().map_err(|_| RfSwitchRx)?,
            None => (),
        };
        match &mut self.rf_switch_tx {
            Some(pin) => pin.set_high().map_err(|_| RfSwitchTx),
            None => Ok(()),
        }
    }
    async fn disable_rf_switch(&mut self) -> Result<(), RadioError> {
        match &mut self.rf_switch_rx {
            Some(pin) => pin.set_low().map_err(|_| RfSwitchRx)?,
            None => (),
        };
        match &mut self.rf_switch_tx {
            Some(pin) => pin.set_low().map_err(|_| RfSwitchTx),
            None => Ok(()),
        }
    }
}
