#[allow(non_upper_case_globals)]
#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
#[allow(dead_code)]
mod sys {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

use libc::{c_char, nl_item, nl_langinfo};
use sys::{_NL_PAPER_HEIGHT, _NL_PAPER_WIDTH};

use crate::{PaperSize, Unit};

fn paper_info(item: nl_item) -> u32 {
    let pointer = unsafe { nl_langinfo(item) } as usize;
    let bytes = pointer.to_ne_bytes();
    u32::from_ne_bytes(bytes[..4].try_into().unwrap())
}

pub fn locale_paper_size() -> Option<PaperSize> {
    if !unsafe { libc::setlocale(libc::LC_PAPER, &[0i8] as *const c_char) }.is_null() {
        let width = paper_info(_NL_PAPER_WIDTH as nl_item);
        let height = paper_info(_NL_PAPER_HEIGHT as nl_item);
        Some(PaperSize::new(
            width as f64,
            height as f64,
            Unit::Millimeter,
        ))
    } else {
        None
    }
}
