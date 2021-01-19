include!(concat!(env!("CARGO_MANIFEST_DIR"), "./quad/main.rs"));

#[cfg(target_os = "android")]
#[cfg_attr(target_os = "android", ndk_glue::main(backtrace = "full"))]
fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
    dbg!(manifest);

    #[cfg(not(feature = "gl",))]
    eprintln!(
        "This example will run only on CI."
    );

    {
        println!("Waiting for NativeScreen");
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

    android_logger::init_once(android_logger::Config::default().with_min_level(log::Level::Trace));

    run();
}
