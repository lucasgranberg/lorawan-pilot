use core::convert::Infallible;

use embassy_stm32::flash::{Bank1Region, Blocking, Flash, MAX_ERASE_SIZE};
use embassy_stm32::gpio::{Level, Output, Pin, Speed};
use embassy_stm32::pac;
use embassy_stm32::peripherals::RNG;
use embassy_stm32::rng::Rng;
use embassy_stm32::spi::Spi;
use embassy_stm32::{bind_interrupts, Peripherals};
use embassy_time::Delay;
use lora_phy::sx126x::{self, Stm32wl, Sx126x, TcxoCtrlVoltage};
use lora_phy::LoRa;
use lorawan::device::non_volatile_store::NonVolatileStore;
use lorawan::device::{Device, DeviceSpecs};
use lorawan::mac::types::Storable;
use postcard::{from_bytes, to_slice};

use crate::iv::{InterruptHandler, Stm32wlInterfaceVariant, SubghzSpiDevice};
use crate::lora_radio::{LoraRadioKind, LoraType};
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
    radio: LoraType<'d>,
    timer: LoraTimer,
    non_volatile_store: DeviceNonVolatileStore<'d>,
}
impl<'a> LoraDevice<'a> {
    pub async fn new(peripherals: Peripherals) -> LoraDevice<'a> {
        let lora: LoraType<'a> = {
            let spi =
                Spi::new_subghz(peripherals.SUBGHZSPI, peripherals.DMA1_CH2, peripherals.DMA1_CH3);
            let spi = SubghzSpiDevice(spi);
            let iv = Stm32wlInterfaceVariant::new(
                Irqs,
                None,
                Some(Output::new(peripherals.PC4.degrade(), Level::Low, Speed::High)),
            )
            .unwrap();
            let config = sx126x::Config {
                chip: Stm32wl { use_high_power_pa: true },
                tcxo_ctrl: Some(TcxoCtrlVoltage::Ctrl1V7),
                use_dcdc: true,
                rx_boost: false,
            };
            LoRa::new(Sx126x::new(spi, iv, config), true, Delay).await.unwrap()
        };
        let non_volatile_store = DeviceNonVolatileStore::new(
            Flash::new_blocking(peripherals.FLASH).into_blocking_regions().bank1_region,
        );
        let ret = Self {
            rng: DeviceRng(Rng::new(peripherals.RNG, Irqs)),
            radio: lora,
            timer: LoraTimer::new(),
            non_volatile_store,
        };
        ret
    }
}
impl defmt::Format for LoraDevice<'_> {
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
        Self { flash, buf: [0xFF; 256] }
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
impl NonVolatileStore for DeviceNonVolatileStore<'_> {
    type Error = NonVolatileStoreError;

    fn save(&mut self, storable: Storable) -> Result<(), Self::Error> {
        self.flash
            .blocking_erase(Self::offset(), Self::offset() + MAX_ERASE_SIZE as u32)
            .map_err(NonVolatileStoreError::Flash)?;
        to_slice(&storable, self.buf.as_mut_slice())
            .map_err(|_| NonVolatileStoreError::Encoding)?;
        self.flash.blocking_write(Self::offset(), &self.buf).map_err(NonVolatileStoreError::Flash)
    }

    fn load(&mut self) -> Result<Storable, Self::Error> {
        self.flash
            .blocking_read(Self::offset(), self.buf.as_mut_slice())
            .map_err(NonVolatileStoreError::Flash)?;
        from_bytes(self.buf.as_mut_slice()).map_err(|_| NonVolatileStoreError::Encoding)
    }
}

impl lorawan::device::rng::Rng for DeviceRng<'_> {
    type Error = Infallible;

    fn next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(self.0.next_u32())
    }
}
impl DeviceSpecs for LoraDevice<'_> {}
impl<'a> Device for LoraDevice<'a> {
    type Timer = LoraTimer;

    type RadioKind = LoraRadioKind<'a>;

    type Rng = DeviceRng<'a>;

    type NonVolatileStore = DeviceNonVolatileStore<'a>;

    type Delay = Delay;

    fn timer(&mut self) -> &mut Self::Timer {
        &mut self.timer
    }

    fn rng(&mut self) -> &mut Self::Rng {
        &mut self.rng
    }

    fn non_volatile_store(&mut self) -> &mut Self::NonVolatileStore {
        &mut self.non_volatile_store
    }

    fn radio(&mut self) -> &mut LoraType<'a> {
        &mut self.radio
    }
}
