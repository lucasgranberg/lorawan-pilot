use embassy_lora::iv::Stm32wlInterfaceVariant;
use embassy_stm32::gpio::Output;
use embassy_stm32::peripherals::{DMA1_CH2, DMA1_CH3, PC4, SUBGHZSPI};
use embassy_stm32::spi::Spi;
use embassy_time::Delay;
use lora_phy::mod_params::RadioError;
use lora_phy::sx1261_2::SX1261_2;
use lora_phy::LoRa;
use lorawan::device::radio::types::RxQuality;
use lorawan::device::radio::Radio;

/// LoRa radio using the physical layer API in the external lora-phy crate
pub struct LoRaRadio<'d> {
    #[allow(clippy::type_complexity)]
    pub(crate) lora:
        LoRa<SX1261_2<Spi<'d, SUBGHZSPI, DMA1_CH2, DMA1_CH3>, Stm32wlInterfaceVariant<Output<'d, PC4>>>, Delay>,
}

impl<'d> LoRaRadio<'d> {
    #[allow(clippy::type_complexity)]
    pub fn new(
        lora: LoRa<SX1261_2<Spi<'d, SUBGHZSPI, DMA1_CH2, DMA1_CH3>, Stm32wlInterfaceVariant<Output<'d, PC4>>>, Delay>,
    ) -> Self {
        Self { lora }
    }
}

/// Provide the LoRa physical layer rx/tx interface for boards supported by the external lora-phy crate
impl<'d> Radio for LoRaRadio<'d> {
    type Error = RadioError;

    async fn tx(
        &mut self,
        config: lorawan::device::radio::types::TxConfig,
        buf: &[u8],
    ) -> Result<usize, <LoRaRadio<'d> as lorawan::device::radio::Radio>::Error> {
        let sf = config.rf.data_rate.spreading_factor.into();
        let bw = config.rf.data_rate.bandwidth.into();
        let cr = config.rf.coding_rate.into();
        let mdltn_params = self.lora.create_modulation_params(sf, bw, cr, config.rf.frequency)?;
        let mut tx_pkt_params = self.lora.create_tx_packet_params(8, false, true, false, &mdltn_params)?;
        self.lora.prepare_for_tx(&mdltn_params, config.pw.into(), false).await?;
        self.lora.tx(&mdltn_params, &mut tx_pkt_params, buf, 0xffffff).await?;
        Ok(0)
    }

    async fn rx(
        &mut self,
        config: lorawan::device::radio::types::RfConfig,
        window_in_secs: u8,
        rx_buf: &mut [u8],
    ) -> Result<
        (usize, lorawan::device::radio::types::RxQuality),
        <LoRaRadio<'d> as lorawan::device::radio::Radio>::Error,
    > {
        let sf = config.data_rate.spreading_factor.into();
        let bw = config.data_rate.bandwidth.into();
        let cr = config.coding_rate.into();
        let mdltn_params = self.lora.create_modulation_params(sf, bw, cr, config.frequency)?;
        let rx_pkt_params =
            self.lora.create_rx_packet_params(8, false, rx_buf.len() as u8, true, true, &mdltn_params)?;
        self.lora.prepare_for_rx(&mdltn_params, &rx_pkt_params, Some(window_in_secs), None, true).await?;
        match self.lora.rx(&rx_pkt_params, rx_buf).await {
            Ok((received_len, rx_pkt_status)) => {
                Ok((
                    received_len as usize,
                    RxQuality::new(rx_pkt_status.rssi, rx_pkt_status.snr as i8), // downcast snr
                ))
            }
            Err(err) => Err(err),
        }
    }

    async fn sleep(
        &mut self,
        _warm_start: bool,
    ) -> Result<(), <LoRaRadio<'d> as lorawan::device::radio::Radio>::Error> {
        self.lora.sleep(false).await
    }
}
