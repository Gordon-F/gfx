include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../quad/main.rs"));

#[cfg(target_os = "android")]
#[cfg_attr(target_os = "android", ndk_glue::main(logger(level = "trace")))]
fn main() {
    /*
    {
        log::info!("App started. Waiting for NativeScreen");
        loop {
            match ndk_glue::native_window().as_ref() {
                Some(_) => {
                    log::info!("NativeScreen Found:{:?}", ndk_glue::native_window());
                    break;
                }
                None => (),
            }
        }
    }
    */
    run();
}
