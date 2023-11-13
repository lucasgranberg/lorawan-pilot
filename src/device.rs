use core::convert::Infallible;

use embassy_stm32::flash::{Bank1Region, Blocking, Flash, MAX_ERASE_SIZE};
use embassy_stm32::gpio::{Level, Output, Pin, Speed};
use embassy_stm32::peripherals::RNG;
use embassy_stm32::rng::Rng;
use embassy_stm32::{bind_interrupts, pac, Peripherals};
use embassy_time::Delay;
use lora_phy::sx1261_2::{Sx126xVariant, SX1261_2};
use lora_phy::LoRa;
use lorawan::device::non_volatile_store::NonVolatileStore;
use lorawan::device::Device;
use lorawan::mac::types::Storable;
use postcard::{from_bytes, to_slice};

use crate::iv::{InterruptHandler, Stm32wlInterfaceVariant, SubGhzSpiDevice};
use crate::lora_radio::{LoRaRadio, LoraType};
use crate::timer::LoraTimer;
use rand_core::RngCore;

bind_interrupts!(struct Irqs{
    SUBGHZ_RADIO => InterruptHandler;
    RNG => embassy_stm32::rng::InterruptHandler<RNG>;
});

extern "C" {
    static __storage: u8;
}
pub struct LoraDevice<'d> {
    rng: DeviceRng<'d>,
    radio: LoRaRadio<'d>,
    timer: LoraTimer,
    non_volatile_store: DeviceNonVolatileStore<'d>,
}
impl<'a> LoraDevice<'a> {
    pub async fn new(peripherals: Peripherals) -> LoraDevice<'a> {
        let lora: LoraType<'a> = {
            let spi = SubGhzSpiDevice::new(
                peripherals.SUBGHZSPI,
                peripherals.DMA1_CH2,
                peripherals.DMA1_CH3,
            );
            let iv = Stm32wlInterfaceVariant::new(
                Irqs,
                None,
                Some(Output::new(
                    peripherals.PC4.degrade(),
                    Level::Low,
                    Speed::High,
                )),
            )
            .unwrap();

            LoRa::new(
                SX1261_2::new(
                    spi,
                    iv,
                    lora_phy::sx1261_2::Config {
                        chip: Sx126xVariant::Stm32wl,
                        txco_ctrl: None,
                    },
                ),
                true,
                Delay,
            )
            .await
            .unwrap()
        };
        let radio = LoRaRadio::new(lora);
        let non_volatile_store = DeviceNonVolatileStore::new(
            Flash::new_blocking(peripherals.FLASH)
                .into_blocking_regions()
                .bank1_region,
        );
        let ret = Self {
            rng: DeviceRng(Rng::new(peripherals.RNG, Irqs)),
            radio,
            timer: LoraTimer::new(),
            non_volatile_store,
        };
        ret
    }
}
impl<'d> defmt::Format for LoraDevice<'d> {
    fn format(&self, fmt: defmt::Formatter<'_>) {
        defmt::write!(fmt, "LoraDevice")
    }
}
pub struct DeviceRng<'a>(Rng<'a, RNG>);

pub struct DeviceNonVolatileStore<'a> {
    flash: Bank1Region<'a, Blocking>,
    buf: [u8; 256],
}
impl<'a> DeviceNonVolatileStore<'a> {
    pub fn new(flash: Bank1Region<'a, Blocking>) -> Self {
        Self {
            flash,
            buf: [0xFF; 256],
        }
    }
    pub fn offset() -> u32 {
        (unsafe { &__storage as *const u8 as u32 }) - pac::FLASH_BASE as u32
    }
}
#[derive(Debug, PartialEq, defmt::Format)]
pub enum NonVolatileStoreError {
    Flash(embassy_stm32::flash::Error),
    Encoding,
}
impl<'m> NonVolatileStore for DeviceNonVolatileStore<'m> {
    type Error = NonVolatileStoreError;

    fn save(&mut self, storable: Storable) -> Result<(), Self::Error> {
        self.flash
            .blocking_erase(Self::offset(), Self::offset() + MAX_ERASE_SIZE as u32)
            .map_err(NonVolatileStoreError::Flash)?;
        to_slice(&storable, self.buf.as_mut_slice())
            .map_err(|_| NonVolatileStoreError::Encoding)?;
        self.flash
            .blocking_write(Self::offset(), &self.buf)
            .map_err(NonVolatileStoreError::Flash)
    }

    fn load(&mut self) -> Result<Storable, Self::Error> {
        self.flash
            .blocking_read(Self::offset(), self.buf.as_mut_slice())
            .map_err(NonVolatileStoreError::Flash)?;
        from_bytes(self.buf.as_mut_slice()).map_err(|_| NonVolatileStoreError::Encoding)
    }
}

impl<'a> lorawan::device::rng::Rng for DeviceRng<'a> {
    type Error = Infallible;

    fn next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(self.0.next_u32())
    }
}
impl<'a> Device for LoraDevice<'a> {
    type Timer = LoraTimer;

    type Radio = LoRaRadio<'a>;

    type Rng = DeviceRng<'a>;

    type NonVolatileStore = DeviceNonVolatileStore<'a>;

    fn timer(&mut self) -> &mut Self::Timer {
        &mut self.timer
    }

    fn radio(&mut self) -> &mut Self::Radio {
        &mut self.radio
    }

    fn rng(&mut self) -> &mut Self::Rng {
        &mut self.rng
    }

    fn non_volatile_store(&mut self) -> &mut Self::NonVolatileStore {
        &mut self.non_volatile_store
    }

    fn max_eirp() -> u8 {
        22
    }

    fn adaptive_data_rate_enabled(&self) -> bool {
        true
    }

    fn handle_device_time(&mut self, _seconds: u32, _nano_seconds: u32) {
        // default do nothing
    }

    fn handle_link_check(&mut self, _gateway_count: u8, _margin: u8) {
        // default do nothing
    }

    fn battery_level(&self) -> Option<f32> {
        None
    }
}
