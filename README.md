# LoRaWAN Demo Application for a TTN gateway under US915 using an STM32WL board

### Description

Demonstrates a basic LoRaWAN application for:

- <a href="https://github.com/lucasgranberg/lorawan">LoRaWAN Class A</a>
- <a href="https://github.com/embassy-rs/lora-phy">LoRa physical layer</a>
- <a href="https://github.com/embassy-rs/embassy">Embassy embedded framework</a>

with the following test setup:

- <a href="https://www.thethingsindustries.com/docs/gateways/models/thethingsindoorgateway/">8 channel TTN gateway</a>
- <a href="https://resources.lora-alliance.org/technical-specifications/rp002-1-0-4-regional-parameters">US915 regional parameters</a>
- <a href="https://www.st.com/en/microcontrollers-microprocessors/stm32wl-series.html">STM32WL with a Semtech SX1262 LoRa chip</a>

The Cargo.toml patch.crates-io specification includes lorawan and embassy repositories and versions that are known to work for this example.

### Code modifications needed

- in your local lorawan-pilot clone, git checkout the ttn-us915-stm32wl branch.
- in main.rs, set the dev_eui, app_eui, and app_key configured for your device and gateway.
- if using an 8 channel gateway without access to other gateways with additional channel support for the US915 fixed channel plan, consider setting a preferred channel block for the first join attempt in the device.rs preferred_join_channel_block_index() function.  The TTN gateway mentioned above uses the second channel block, indicated in the function as zero-based index 1.  The LoRaWAN implementation will attempt to join using a random channel from each channel block; preferred_join_channel_block_index() simply prioritizes the order of attempts.

### Suggested setup

The following can all be run from your local lorawan-pilot folder, with the ttn-us915-stm32wl branch checked out:

- rustup update (if needed)
- cargo clean
- cargo update
- plug in the stm32wl device
- DEFMT_LOG=info cargo run --release

### Chat

A public chat on LoRa/LoRaWAN topics using Rust is here:

- <a href="https://matrix.to/#/#public-lora-wan-rs:matrix.org">Matrix room</a>
