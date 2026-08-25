//! Decode `STACKS` RingBuf records (Phase 12).

use obsagent_common::{
    EventKind, StackSampleEvent, STACK_SAMPLE_EVENT_SIZE, STACK_SAMPLE_MAX_FRAMES,
};

pub fn decode_stack_sample(bytes: &[u8]) -> Option<StackSampleEvent> {
    if bytes.len() < STACK_SAMPLE_EVENT_SIZE {
        return None;
    }
    if EventKind::from_u8(*bytes.first()?) != Some(EventKind::StackSample) {
        return None;
    }
    let ev = unsafe { (bytes.as_ptr() as *const StackSampleEvent).read_unaligned() };
    if ev.frame_count as usize > STACK_SAMPLE_MAX_FRAMES {
        return None;
    }
    Some(ev)
}

#[cfg(test)]
mod tests {
    use super::*;
    use obsagent_common::EventKind;

    #[test]
    fn roundtrip_layout() {
        let ev = StackSampleEvent {
            kind: EventKind::StackSample as u8,
            _pad0: [0; 7],
            tgid: 42,
            pid: 43,
            ts_ns: 9_000,
            frame_count: 2,
            _pad1: [0; 3],
            ips: [0x1000, 0x2000, 0, 0, 0, 0, 0, 0],
        };
        let bytes =
            unsafe { core::slice::from_raw_parts(&ev as *const _ as *const u8, STACK_SAMPLE_EVENT_SIZE) };
        let got = decode_stack_sample(bytes).unwrap();
        assert_eq!(got.tgid, 42);
        assert_eq!(got.frame_count, 2);
        assert_eq!(got.ips[0], 0x1000);
    }
}
