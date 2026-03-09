use embassy_executor::task;
use embassy_time::Timer;
use embassy_usb::class::cdc_acm::{CdcAcmClass, State as CdcState};
use embassy_usb::{Builder, Config as UsbDeviceConfig};
use embassy_usb::class::hid::{HidReaderWriter, State as HidState};
use embassy_futures::join::join3;
use static_cell::StaticCell;

use esp_hal::otg_fs::{Usb, asynch::Driver as EspUsbDriver, asynch::Config as EspUsbConfig};

use crate::services::hid::{HID_CH, HID_REPORT_MAP};
use crate::services::usb_logger::LOGGER_PIPE;

static HID_STATE: StaticCell<HidState> = StaticCell::new();
static CDC_STATE: StaticCell<CdcState> = StaticCell::new();
static EP_OUT_BUFFER: StaticCell<[u8; 256]> = StaticCell::new();
static CONFIG_DESC: StaticCell<[u8; 256]> = StaticCell::new();
static BOS_DESC: StaticCell<[u8; 256]> = StaticCell::new();
static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();

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
    config.product = Some("BitBandV2 Debug"); 
    config.self_powered = true;
    config.max_power = 100; // Increased for composite device

    let mut builder = Builder::new(
        driver,
        config,
        config_descriptor,
        bos_descriptor,
        &mut [],
        control_buf,
    );

    // 1. Setup CDC ACM (Serial Logger)
    let cdc_state = CDC_STATE.init(CdcState::new());
    let mut class = CdcAcmClass::new(&mut builder, cdc_state, 64);

    // 2. Setup HID Keyboard
    let hid_config = embassy_usb::class::hid::Config {
        report_descriptor: HID_REPORT_MAP,
        request_handler: None,
        poll_ms: 1,
        max_packet_size: 8,
    };

    let hid_state = HID_STATE.init(HidState::new());
    let hid = HidReaderWriter::<_, 1, 8>::new(&mut builder, hid_state, hid_config);
    
    let mut usb = builder.build();
    let (_, mut writer) = hid.split();

    // Task A: USB Stack
    let usb_fut = usb.run();

    // Task B: HID Writer
    let hid_fut = async {
        loop {
            let report = HID_CH.receive().await;
            let packet = [
                report.modifier, 0,
                report.keys[0], report.keys[1], report.keys[2],
                report.keys[3], report.keys[4], report.keys[5],
            ];

            if let Err(_) = writer.write(&packet).await {
                Timer::after_millis(100).await;
            }
        }
    };

    // Task C: CDC Logger (Drains the Pipe to Serial)
    let cdc_fut = async {
        let mut rx_buf = [0u8; 64];
        loop {
            class.wait_connection().await;
            loop {
                let n = LOGGER_PIPE.read(&mut rx_buf).await;
                if class.write_packet(&rx_buf[..n]).await.is_err() {
                    break;
                }
            }
        }
    };

    // Run all three concurrently
    join3(usb_fut, hid_fut, cdc_fut).await;
}
