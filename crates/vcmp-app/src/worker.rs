use std::any::Any;
use std::panic::{self, AssertUnwindSafe};

pub fn catch_panic<T>(work: impl FnOnce() -> T) -> Result<T, String> {
    panic::catch_unwind(AssertUnwindSafe(work)).map_err(|payload| {
        let message = panic_message(payload.as_ref());
        log::error!("a background job stopped: {message}");
        message
    })
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    let text = payload
        .downcast_ref::<&str>()
        .map(|text| text.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown cause".into());
    format!("internal error: {text}")
}

#[cfg(test)]
mod tests {
    use super::catch_panic;

    #[test]
    fn a_panic_becomes_an_error_with_its_message() {
        assert_eq!(catch_panic(|| 7), Ok(7));
        let error = catch_panic(|| -> i32 { panic!("bad frame") }).unwrap_err();
        assert!(error.contains("bad frame"), "{error}");
        let owned = String::from("owned");
        let error = catch_panic(move || -> i32 { panic!("{owned}") }).unwrap_err();
        assert!(error.contains("owned"), "{error}");
    }
}
