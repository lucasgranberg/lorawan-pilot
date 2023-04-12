use embassy_stm32::gpio::{AnyPin, Output};

pub struct RadioSwitch<'a> {
    ctrl1: Output<'a, AnyPin>,
    ctrl2: Output<'a, AnyPin>,
    ctrl3: Output<'a, AnyPin>,
}
impl<'a> RadioSwitch<'a> {
    pub fn new(
        ctrl1: Output<'a, AnyPin>,
        ctrl2: Output<'a, AnyPin>,
        ctrl3: Output<'a, AnyPin>,
    ) -> Self {
        Self {
            ctrl1,
            ctrl2,
            ctrl3,
        }
    }
}
impl<'a> crate::stm32wl::RadioSwitch for RadioSwitch<'a> {
    fn set_rx(&mut self) {
        self.ctrl3.set_high();
        self.ctrl1.set_high();
        self.ctrl2.set_low();
    }

    fn set_tx(&mut self) {
        self.ctrl3.set_high();
        self.ctrl1.set_low();
        self.ctrl2.set_high();
    }
}
impl<'a> Drop for RadioSwitch<'a> {
    fn drop(&mut self) {
        self.ctrl1.set_low();
        self.ctrl2.set_low();
        self.ctrl3.set_low();
    }
}
