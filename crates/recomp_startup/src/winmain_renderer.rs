//! Phase 04 row 04-F: WinMain 0040E865..0040E88E: the renderer creation, the load-direct hint and the two renderer
//! notification registrations.

use recomp_game::gamebryo::renderer::{
    ORIGINAL_RETURN_TRUE_STUB, ORIGINAL_TRANSITION_CALLBACK, RendererCallbackPairs,
};

/// IDA: the two 00B3F928 renderer methods WinMain calls: 0040C280 (lost device, +0xAB8/+0xAC8) and 0040C1B0 (reset,
/// +0xA98/+0xAA8). Each returns the index. Completion: `RendererCallbackPairs` implements it; the recording test double stands in where only the order matters.
pub trait RendererCallbackRegistry {
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn add_lost_device(&mut self, function: u32, data: u32) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn add_reset(&mut self, function: u32, data: u32) -> u32;
}

impl RendererCallbackRegistry for RendererCallbackPairs {
    /// IDA: N/A `RendererCallbackPairs`'s side of `RendererCallbackRegistry`.
    fn add_lost_device(&mut self, function: u32, data: u32) -> u32 {
        Self::add_lost_device(self, function, data)
    }
    /// IDA: N/A `RendererCallbackPairs`'s side of `RendererCallbackRegistry`.
    fn add_reset(&mut self, function: u32, data: u32) -> u32 {
        Self::add_reset(self, function, data)
    }
}

/// IDA: 0040E86A..0040E88E after the renderer: 0040E876 byte 00B3F709 = 1, then the two registrations in order (lost device
/// first, 0040E87D; reset, 0040E88E). Completion: 100%.
/// TODO: [ ] Carried: 00405440 (4,308-function closure) with its consuming phase; 0076A970/0076B3F0 invoke the arrays on
///       reset/lost device.
pub fn register_winmain_renderer_callbacks(
    pairs: &mut dyn RendererCallbackRegistry,
    load_direct_hint: &mut bool,
) {
    *load_direct_hint = true;
    pairs.add_lost_device(ORIGINAL_RETURN_TRUE_STUB, 0);
    pairs.add_reset(ORIGINAL_TRANSITION_CALLBACK, 0);
}

/// IDA: 0040E865..0040E88E. `create_renderer` is 0040E865 -> 004052F0 -> 00498340; it returns only on success in the
/// original, since 004052F0's failure path shows the box and calls ExitProcess(0). The host's `Err` is that path: it skips
/// the stores, and the caller unwinds and exits 0. Completion: 100% of the order; the creation is the renderer port's.
/// # Errors
/// What `create_renderer` returned, standing for 004052F0's failure exit.
pub fn run_winmain_renderer_block<R: RendererCallbackRegistry, E>(
    create_renderer: impl FnOnce() -> Result<R, E>,
    load_direct_hint: &mut bool,
) -> Result<R, E> {
    let mut renderer = create_renderer()?;
    register_winmain_renderer_callbacks(&mut renderer, load_direct_hint);
    Ok(renderer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default, PartialEq, Eq)]
    struct RecordedPairs {
        calls: Vec<(&'static str, u32, u32)>,
    }

    impl RendererCallbackRegistry for RecordedPairs {
        fn add_lost_device(&mut self, function: u32, data: u32) -> u32 {
            self.calls.push(("lost", function, data));
            0
        }
        fn add_reset(&mut self, function: u32, data: u32) -> u32 {
            self.calls.push(("reset", function, data));
            0
        }
    }

    /// The registration half of the C++ `verify_callback_pairs`: lost device first, and the hint. The arrays' growth and
    /// removal are in the renderer port's half.
    #[test]
    fn registrations_are_in_original_order() {
        let mut hint = false;
        let renderer =
            run_winmain_renderer_block(|| Ok::<_, ()>(RecordedPairs::default()), &mut hint)
                .unwrap();
        assert!(hint, "00B3F709 must be set");
        assert_eq!(
            renderer.calls,
            [("lost", 0x0097_7C50, 0), ("reset", 0x0040_5440, 0)]
        );
    }

    /// The real arrays: the one index each, in their own pair.
    #[test]
    fn registrations_reach_the_renderer_arrays() {
        let mut hint = false;
        let pairs =
            run_winmain_renderer_block(|| Ok::<_, ()>(RendererCallbackPairs::default()), &mut hint)
                .unwrap();
        assert_eq!(
            pairs.lost_device_functions.slots[0],
            ORIGINAL_RETURN_TRUE_STUB
        );
        assert_eq!(pairs.reset_functions.slots[0], ORIGINAL_TRANSITION_CALLBACK);
    }

    /// 004052F0's failure exit never reaches 0040E86A.
    #[test]
    fn a_failed_renderer_stops_before_the_stores() {
        let mut hint = false;
        let failed = run_winmain_renderer_block::<RecordedPairs, _>(|| Err("no device"), &mut hint);
        assert_eq!(failed, Err("no device"));
        assert!(!hint);
    }
}
