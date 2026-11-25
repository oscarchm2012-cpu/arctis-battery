//! Probe SteelSeries HID interfaces: `cargo run --example probe`
fn main() {
    let api = hidapi::HidApi::new().expect("hidapi");
    for d in api.device_list().filter(|d| d.vendor_id() == 0x1038) {
        println!(
            "pid={:04x} if={} page={:04x} usage={:04x} product={:?} path={:?}",
            d.product_id(), d.interface_number(), d.usage_page(), d.usage(),
            d.product_string(), d.path()
        );
        if d.usage_page() != 0xffc0 {
            continue;
        }
        let Ok(dev) = d.open_device(&api) else { println!("  open failed"); continue };
        // One request only: the dongle also carries the headset's audio link.
        let req = [0x00u8, 0xb0];
        let w = dev.write(&req);
        let mut buf = [0u8; 128];
        let n = dev.read_timeout(&mut buf, 1000);
        let len = n.as_ref().map(|v| *v).unwrap_or(0).min(16);
        println!("  write {:?} -> {:?}; read {:?}: {:02x?}", req, w, n, &buf[..len]);
    }
}
