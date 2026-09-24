use runtime_exec::{
    ApiKey, TouchDump, key_name, touch_log_count, touch_log_dump, touch_log_record, touch_log_reset,
};

#[test]
fn mask_dedup_and_reset() {
    touch_log_reset();
    assert_eq!(touch_log_count(), 0);
    touch_log_record(ApiKey::USER_AGENT);
    touch_log_record(ApiKey::NOW);
    touch_log_record(ApiKey::USER_AGENT);
    assert_eq!(touch_log_count(), 2, "дубликат занимает один бит");
    touch_log_reset();
    assert_eq!(touch_log_count(), 0, "сброс чистит маску, capacity живёт");
    touch_log_record(ApiKey::NOW);
    assert_eq!(touch_log_count(), 1);
}

#[test]
fn dump_renders_names_and_task_seq() {
    touch_log_reset();
    touch_log_record(ApiKey::USER_AGENT);
    touch_log_record(ApiKey::WEBDRIVER);
    let dump: TouchDump = touch_log_dump();
    assert_eq!(dump.keys.len(), 2);
    let text = dump.to_string();
    assert!(text.contains("navigator.userAgent"));
    assert!(text.contains("navigator.webdriver"));
    assert!(text.contains("2 api touched"));
    let empty: TouchDump = touch_log_dump();
    touch_log_reset();
    let after: TouchDump = touch_log_dump();
    assert!(after.seq > empty.seq, "каждый сброс — новая задача");
    assert_eq!(key_name(ApiKey::USER_AGENT), "navigator.userAgent");
}

#[test]
fn dump_empty_task_reports_none() {
    touch_log_reset();
    let text = touch_log_dump().to_string();
    assert!(
        text.contains("none"),
        "пустая задача не рождает имён: {text}"
    );
    assert_eq!(touch_log_count(), 0);
}
