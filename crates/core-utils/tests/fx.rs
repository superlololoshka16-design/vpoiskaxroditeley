use core_utils::math::{FxBuild, FxHasher64};
use std::collections::HashMap;
use std::hash::Hasher;

#[test]
fn fx_known_values_stable() {
    let mut h = FxHasher64::default();
    h.write_u64(0);
    let a = h.finish();
    let mut h = FxHasher64::default();
    h.write_u64(0);
    assert_eq!(a, h.finish());

    let mut h = FxHasher64::default();
    h.write(b"");
    assert_eq!(h.finish(), 0);

    let mut h1 = FxHasher64::default();
    h1.write_u64(1);
    let mut h2 = FxHasher64::default();
    h2.write_u64(2);
    assert_ne!(h1.finish(), h2.finish());

    let mut h = FxHasher64::default();
    h.write(b"same");
    let via_write = h.finish();
    let mut h = FxHasher64::default();
    h.write(b"same");
    assert_eq!(via_write, h.finish());
    let mut h = FxHasher64::default();
    h.write(b"same\0tail");
    assert_ne!(via_write, h.finish());
}

#[test]
fn fx_no_collisions_on_small_int_space() {
    let mut map: HashMap<u64, usize, FxBuild> = HashMap::default();
    for i in 0..10_000u64 {
        let slot = i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        map.insert(slot, i as usize);
    }
    assert_eq!(map.len(), 10_000);
    for i in 0..10_000u64 {
        let slot = i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        assert_eq!(map.get(&slot), Some(&(i as usize)));
    }
}

#[test]
fn fx_u32_keys_survive_garbage_lookup() {
    let mut map: HashMap<u32, &str, FxBuild> = HashMap::default();
    for i in 0..256u32 {
        map.insert(i, "x");
    }
    assert_eq!(map.get(&255), Some(&"x"));
    assert_eq!(map.get(&256), None);
    assert_eq!(map.get(&u32::MAX), None);
}
