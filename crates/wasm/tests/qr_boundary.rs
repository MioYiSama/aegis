#![cfg(target_arch = "wasm32")]
use aegis_wasm::{decode_chroma_rgba, encode_chroma_png};
use wasm_bindgen_test::wasm_bindgen_test;
use wasm_bindgen::JsCast;

#[wasm_bindgen_test]
fn rejects_invalid_image_boundaries() {
    for (bytes, width, height) in [(vec![255; 3], 1, 1), (vec![], 0, 1), (vec![], 1, 0), (vec![], 1921, 1), (vec![], 1, 1921)] {
        let error = decode_chroma_rgba(&bytes, width, height).unwrap_err();
        assert_eq!(js_message(error), "invalid_image");
    }
}
#[wasm_bindgen_test]
fn white_pixel_is_unreadable() {
    assert_eq!(decode_chroma_rgba(&[255; 4], 1, 1).unwrap(), None);
}
#[wasm_bindgen_test]
fn rejects_invalid_payloads() {
    for payload in ["", "中文", "abcdefghijklmnopqrstuvwxyz01234567890123456"] {
        assert_eq!(js_message(encode_chroma_png(payload).unwrap_err()), "invalid_payload");
    }
}
fn js_message(error: wasm_bindgen::JsError) -> String {
    let value: wasm_bindgen::JsValue = error.into();
    wasm_bindgen::JsValue::from(value).unchecked_into::<js_sys::Error>().message().into()
}
