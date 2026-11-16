//! CRX3 packages from anywhere: verification must refuse, never panic or
//! hang (spec 09 §3).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = mado_extensions::verify(
        data,
        mado_extensions::Expect {
            id: None,
            from_store: true,
        extra_publishers: &[],
        },
    );
});
