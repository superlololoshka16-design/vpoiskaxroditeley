use runtime_exec::NormCache;
use std::sync::Arc;
use std::thread;

#[test]
fn admission_is_bounded_without_sweeping() {
    let cache = NormCache::with_budget(4, 16384);
    for key in 0..1000 {
        cache.put_raw(1, key, key, Arc::new(smallvec::SmallVec::new()));
        cache.put_src(1, key, Arc::from("function(){}"));
    }
    assert_eq!(cache.raw_len(), 4);
    assert_eq!(cache.src_len(), 4);
    assert!(cache.accounted_bytes() <= 16384);
}

#[test]
fn oversized_source_is_not_admitted() {
    let cache = NormCache::with_budget(4, 1024);
    cache.put_src(1, 1, Arc::from("x".repeat(1024)));
    assert_eq!(cache.src_len(), 0);
    assert_eq!(cache.accounted_bytes(), 0);
}

#[test]
fn raw_entry_shares_the_execution_arguments() {
    let cache = NormCache::with_budget(4, 16384);
    let args: Arc<smallvec::SmallVec<[runtime_exec::Lit; 16]>> =
        Arc::new(smallvec::SmallVec::new());
    cache.put_raw(1, 2, 3, Arc::clone(&args));
    let (_, cached) = cache.lookup_raw(1, 2).unwrap();
    assert!(Arc::ptr_eq(&args, &cached));
}

#[test]
fn concurrent_admission_cannot_exceed_capacity() {
    let cache = Arc::new(NormCache::with_budget(8, 32768));
    thread::scope(|scope| {
        for thread in 0..8u64 {
            let cache = Arc::clone(&cache);
            scope.spawn(move || {
                for key in 0..100 {
                    cache.put_raw(thread, key, key, Arc::new(smallvec::SmallVec::new()));
                    cache.put_src(thread, key, Arc::from("function(){}"));
                }
            });
        }
    });
    assert!(cache.raw_len() <= 8);
    assert!(cache.src_len() <= 8);
    assert!(cache.accounted_bytes() <= 32768);
}

#[test]
fn failed_duplicate_insert_releases_its_reservation() {
    let cache = NormCache::with_budget(4, 16384);
    cache.put_src(1, 2, Arc::from("first"));
    let before = cache.accounted_bytes();
    cache.put_src(1, 2, Arc::from("second"));
    assert_eq!(cache.accounted_bytes(), before);
    assert_eq!(cache.lookup_src(1, 2).unwrap().as_ref(), "first");
}

#[test]
fn sweep_evicts_idle_entries() {
    let cache = NormCache::with_budget(16, 65536);
    cache.put_src(1, 2, Arc::from("first"));
    cache.sweep(0);
    assert_eq!(cache.src_len(), 0);
    assert!(cache.lookup_src(1, 2).is_none());
}
