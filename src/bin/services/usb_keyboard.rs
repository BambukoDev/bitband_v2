use embassy_executor::task;
use embassy_time::Timer;
use embassy_usb::class::cdc_acm::CdcAcmClass;
use embassy_usb::{Builder, Config as UsbDeviceConfig};
use embassy_usb::class::hid::{HidReaderWriter, State};
use embassy_usb::driver::EndpointError;
use embassy_futures::join::join;
use static_cell::StaticCell;

use esp_hal::otg_fs::{Usb, asynch::Driver as EspUsbDriver, asynch::Config as EspUsbConfig};

use crate::services::hid::{HID_CH, HID_REPORT_MAP};

static STATE: StaticCell<State> = StaticCell::new();
static EP_OUT_BUFFER: StaticCell<[u8; 256]> = StaticCell::new();
static CONFIG_DESC: StaticCell<[u8; 256]> = StaticCell::new();
static BOS_DESC: StaticCell<[u8; 256]> = StaticCell::new();
static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();
static USB_DRIVER: StaticCell<EspUsbDriver<'static>> = StaticCell::new();

#[task]
pub async fn usb_keyboard_task(usb_peripheral: Usb<'static>) {
    let ep_out_buffer = EP_OUT_BUFFER.init([0; 256]);
    let config_descriptor = CONFIG_DESC.init([0; 256]);
    let bos_descriptor = BOS_DESC.init([0; 256]);
    let control_buf = CONTROL_BUF.init([0; 64]);

    let mut usb_config = EspUsbConfig::default();
    usb_config.vbus_detection = true; 

    let driver = EspUsbDriver::new(usb_peripheral, ep_out_buffer, usb_config);

    let mut config = UsbDeviceConfig::new(0xcafe, 0x2137);
    config.manufacturer = Some("Bambuko");
    config.product = Some("BitBandV2");
    config.max_power = 100;

    let mut builder = Builder::new(
        driver,
        config,
        config_descriptor,
        bos_descriptor,
        &mut [],
        control_buf,
    );

    let hid_config = embassy_usb::class::hid::Config {
        report_descriptor: HID_REPORT_MAP,
        request_handler: None,
        poll_ms: 1,
        max_packet_size: 8,
    };

    let state = STATE.init(State::new());
    let hid = HidReaderWriter::<_, 1, 8>::new(&mut builder, state, hid_config);
    let mut usb = builder.build();
    let (_, mut writer) = hid.split();

    loop {
        join(
            usb.run(),
            async {
                loop {
                    let report = HID_CH.receive().await;
                    let packet = [
                        report.modifier, 0,
                        report.keys[0], report.keys[1], report.keys[2],
                        report.keys[3], report.keys[4], report.keys[5],
                    ];

                    // This will wait/retry if the USB is not ready
                    match writer.write(&packet).await {
                        Ok(_) => (),
                        Err(e) => {
                            Timer::after_millis(500).await;
                        }
                    }
                }
            }
        ).await;
    }
}
