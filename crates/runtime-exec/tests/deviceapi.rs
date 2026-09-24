use bytes::Bytes;
use compact_str::CompactString;
use runtime_exec::{
    FetchReply, WorkerPool, install as bridge_install,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
mod common;
use common::{pool, req};

fn bridge_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

async fn eval(pool: &WorkerPool, script: &str) -> String {
    let o = pool.exec(req(script, 3000)).await;
    o.token
        .unwrap_or_else(|| panic!("js error: {:?}", o.err))
        .as_str()
        .to_string()
}

type JobRecord = (CompactString, CompactString, Vec<u8>, Option<String>);

#[derive(Clone, Default)]
struct SeenJobs {
    inner: Arc<Mutex<Vec<JobRecord>>>,
}

static BRIDGE_SEQ: AtomicUsize = AtomicUsize::new(0);

fn spawn_recording_bridge(seen: SeenJobs) {
    let (tx, rx) = crossbeam_channel::bounded(16);
    bridge_install(tx);
    BRIDGE_SEQ.fetch_add(1, Ordering::SeqCst);
    std::thread::spawn(move || {
        for job in rx {
            let ct = job
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                .map(|(_, v)| v.as_str().to_string());
            seen.inner.lock().expect("lock").push((
                job.url.clone(),
                job.method.clone(),
                job.body.clone().unwrap_or_default().to_vec(),
                ct,
            ));
            let _ = job.reply.send(FetchReply {
                status: 204,
                headers: smallvec::smallvec![],
                set_cookie: smallvec::smallvec![CompactString::const_new("beacon=1; Path=/")],
                body: Bytes::new(),
            });
        }
    });
}

#[tokio::test]
async fn enumerate_devices_is_chrome_shaped_and_deterministic() {
    let pool = pool();
    let out = eval(
        &pool,
        "navigator.mediaDevices.enumerateDevices().then(function (list) {\
             window.__ids = list.map(function (d) { return d.deviceId; }).join(';');\
             window.__grps = list.map(function (d) { return d.groupId; }).join(';');\
         });\
         'kick';",
    )
    .await;
    assert_eq!(out, "kick");
    let ids = eval(&pool, "window.__ids;").await;
    assert!(
        ids.contains("default;communications;default;communications;"),
        "ids: {ids}"
    );
    let grps = eval(&pool, "window.__grps;").await;
    let g: Vec<&str> = grps.split(';').collect();
    assert_eq!(g[0], g[1], "mics share group");
    assert_eq!(g[2], g[3], "speakers share group");
    assert_ne!(g[0], g[4], "camera has its own group");
    let kinds = eval(
        &pool,
        "navigator.mediaDevices.enumerateDevices().then(function (l) {\
             window.__k = [l.length, l.map(function (d) { return d.kind; }).join(','),\
             l.every(function (d) { return d.label === ''; }),\
             /^[0-9a-f]{64}$/.test(l[4].deviceId),\
             Object.prototype.toString.call(l[0]), typeof l[0].toJSON].join('|');\
         });\
         'k2';",
    )
    .await;
    assert_eq!(kinds, "k2");
    let k = eval(&pool, "window.__k;").await;
    assert_eq!(
        k,
        "5|audioinput,audioinput,audiooutput,audiooutput,videoinput|true|true|[object MediaDeviceInfo]|function",
        "devices: {k}"
    );
    let stable = eval(
        &pool,
        "navigator.mediaDevices.enumerateDevices().then(function (a) {\
             navigator.mediaDevices.enumerateDevices().then(function (b) {\
                 window.__same = a[4].deviceId === b[4].deviceId;\
             });\
         });\
         'k3';",
    )
    .await;
    assert_eq!(stable, "k3");
    let same = eval(&pool, "window.__same;").await;
    assert_eq!(same, "true", "ids stable across calls");
}

#[tokio::test]
async fn get_user_media_rejects_like_chrome() {
    let pool = pool();
    let out = eval(
        &pool,
        "navigator.mediaDevices.getUserMedia({ video: true }).then(\
             function () { return 'resolved'; },\
             function (e) { return 'rej:' + e.name + ':' + e.message + ':' + (e instanceof DOMException) + ':' + e.code; }\
         ).then(function (v) { window.__r = v; });\
         'kick';",
    )
    .await;
    assert_eq!(out, "kick");
    let r = eval(&pool, "window.__r;").await;
    assert_eq!(
        r, "rej:NotAllowedError:Permission denied:true:0",
        "gum: {r}"
    );
}

#[tokio::test]
async fn get_user_media_dirty_inputs_throw_type_errors() {
    let pool = pool();
    let out = eval(
        &pool,
        "var out = [];\
         try { navigator.mediaDevices.getUserMedia(); } catch (e) { out.push('none:' + e.name + ':' + (e.message.indexOf(\"not of type '(MediaStreamConstraints or boolean)'\") !== -1)); }\
         try { navigator.mediaDevices.getUserMedia(42); } catch (e) { out.push('num:' + e.name); }\
         try { navigator.mediaDevices.getUserMedia({}); } catch (e) { out.push('empty:' + e.name + ':' + (e.message.indexOf('At least one of audio or video must be requested') !== -1)); }\
         out.join('|');",
    )
    .await;
    assert_eq!(
        out, "none:TypeError:true|num:TypeError|empty:TypeError:true",
        "gum dirty: {out}"
    );
}

#[tokio::test]
async fn permissions_query_states_and_notification_asymmetry() {
    let pool = pool();
    let out = eval(
        &pool,
        "Promise.all([\
             navigator.permissions.query({ name: 'notifications' }),\
             navigator.permissions.query({ name: 'geolocation' }),\
             navigator.permissions.query({ name: 'accelerometer' })\
         ]).then(function (rs) {\
             window.__perm = [rs[0].state, rs[1].state, rs[2].state, rs[0].name,\
                              String(rs[0].onchange), typeof rs[0].addEventListener,\
                              Object.prototype.toString.call(rs[0]),\
                              Notification.permission].join('|');\
         });\
         'q';",
    )
    .await;
    assert_eq!(out, "q");
    let perm = eval(&pool, "window.__perm;").await;
    assert_eq!(
        perm, "prompt|prompt|granted|notifications|null|function|[object PermissionStatus]|default",
        "perm: {perm}"
    );
}

#[tokio::test]
async fn permissions_query_dirty_inputs_throw_chrome_errors() {
    let pool = pool();
    let out = eval(
        &pool,
        "var out = [];\
         try { navigator.permissions.query(5); } catch (e) { out.push('notobj:' + e.name + ':' + (e.message.indexOf(\"not of type 'PermissionsDescriptor'\") !== -1)); }\
         try { navigator.permissions.query({}); } catch (e) { out.push('noname:' + e.name + ':' + (e.message.indexOf(\"Failed to read the 'name'\") !== -1)); }\
         try { navigator.permissions.query({ name: 'bogus-perm' }); } catch (e) { out.push('badname:' + e.name + ':' + (e.message.indexOf(\"is not a valid enum value\") !== -1)); }\
         try { navigator.permissions.query({ name: 7 }); } catch (e) { out.push('namenum:' + e.name); }\
         out.push('hasrequest:' + ('request' in navigator.permissions));\
         out.join('|');",
    )
    .await;
    assert_eq!(
        out,
        "notobj:TypeError:true|noname:TypeError:true|badname:TypeError:true|namenum:TypeError|hasrequest:false",
        "perm dirty: {out}"
    );
}

#[tokio::test]
async fn battery_is_profile_deterministic_and_self_consistent() {
    let pool = pool();
    let out = eval(
        &pool,
        "navigator.getBattery().then(function (b) {\
             window.__b1 = [typeof b.charging, typeof b.level, typeof b.chargingTime, typeof b.dischargingTime,\
                             b.level >= 0 && b.level <= 1,\
                             (b.charging && b.chargingTime !== Infinity && b.dischargingTime === Infinity) ||\
                             (!b.charging && b.dischargingTime !== Infinity && b.chargingTime === Infinity),\
                             Object.prototype.toString.call(b), typeof b.addEventListener,\
                             String(b.onlevelchange)].join('|');\
             return navigator.getBattery();\
         }).then(function (b2) {\
             window.__b2 = [b2.level, b2.charging, b2.chargingTime, b2.dischargingTime].join('|');\
         });\
         'bat';",
    )
    .await;
    assert_eq!(out, "bat");
    let b1 = eval(&pool, "window.__b1;").await;
    assert_eq!(
        b1, "boolean|number|number|number|true|true|[object BatteryManager]|function|null",
        "battery: {b1}"
    );
    let b2 = eval(&pool, "window.__b2;").await;
    assert_eq!(b2, b2, "stable");
    let b3 = eval(
        &pool,
        "navigator.getBattery().then(function (b) { window.__b3 = [b.level, b.charging, b.chargingTime, b.dischargingTime].join('|'); });\
         'b3';",
    )
    .await;
    assert_eq!(b3, "b3");
    let b3v = eval(&pool, "window.__b3;").await;
    assert_eq!(b2, b3v, "battery identical across awaits: {b2} vs {b3v}");
}

#[tokio::test]
async fn beacon_dirty_inputs_throw_or_return_false() {
    let _guard = bridge_lock().lock().expect("lock");
    let pool = pool();
    let out = eval(
        &pool,
        "var out = [];\
         try { navigator.sendBeacon(); } catch (e) { out.push('none:' + e.name + ':' + (e.message.indexOf('1 argument required, but only 0 present') !== -1)); }\
         try { navigator.sendBeacon(42, 'x'); } catch (e) { out.push('num:' + e.name + ':' + (e.message.indexOf(\"not of type '(string or URL)'\") !== -1)); }\
         out.push('js:' + navigator.sendBeacon('javascript:alert(1)', 'x'));\
         out.push('data:' + navigator.sendBeacon('data:text/plain,hi', 'x'));\
         var big = new Array(70000).join('x');\
         out.push('big:' + navigator.sendBeacon('/huge', big));\
         out.push('rel:' + navigator.sendBeacon('/telemetry', 'ping=1'));\
         out.join('|');",
    )
    .await;
    assert_eq!(
        out, "none:TypeError:true|num:TypeError:true|js:false|data:false|big:false|rel:true",
        "beacon dirty: {out}"
    );
}

#[tokio::test]
async fn native_fn_names_and_lengths_survive_len_set() {
    let pool = pool();
    let out = eval(
        &pool,
        "[navigator.sendBeacon.name, navigator.sendBeacon.length,\
         navigator.getBattery.name, navigator.getBattery.length,\
         navigator.permissions.query.name, navigator.permissions.query.length,\
         navigator.mediaDevices.enumerateDevices.name,\
         navigator.mediaDevices.getUserMedia.length].join('|')",
    )
    .await;
    assert_eq!(
        out, "sendBeacon|1|getBattery|0|query|1|enumerateDevices|0",
        "fn names wiped by length set: {out}"
    );
}

#[tokio::test]
async fn beacon_routes_through_branch1_socket_with_cookies() {
    let _guard = bridge_lock().lock().expect("lock");
    let seen = SeenJobs::default();
    spawn_recording_bridge(seen.clone());
    let pool = pool();
    let o = pool
        .exec(req(
            "var r = navigator.sendBeacon('/telemetry', 'ping=1');\
         var r2 = navigator.sendBeacon('https://mock.local/abs', 'body');\
         [r, r2].join('|');",
            3000,
        ))
        .await;
    assert_eq!(
        o.token.as_deref(),
        Some("true|true"),
        "beacon ok: {:?}",
        o.err
    );
    assert_eq!(
        o.cookie_out.as_deref(),
        Some("beacon=1"),
        "set-cookie ingested: {:?}",
        o.cookie_out
    );
    std::thread::sleep(Duration::from_millis(50));
    let jobs = seen.inner.lock().expect("lock").clone();
    assert_eq!(jobs.len(), 2, "bridge jobs: {jobs:?}");
    assert_eq!(
        jobs[0].0, "https://mock.local/telemetry",
        "rel resolved: {jobs:?}"
    );
    assert_eq!(jobs[0].1, "POST");
    assert_eq!(jobs[0].2, b"ping=1".to_vec());
    assert_eq!(jobs[0].3.as_deref(), Some("text/plain;charset=UTF-8"));
    assert_eq!(jobs[1].0, "https://mock.local/abs");
}

#[tokio::test]
async fn dom_exception_class_behaves() {
    let pool = pool();
    let out = eval(
        &pool,
        "var e = new DOMException('Permission denied', 'NotAllowedError');\
         var d = new DOMException('gone', 'NotFoundError');\
         [e.name, e.message, String(e), e instanceof DOMException, d.code, e.code,\
          Object.prototype.toString.call(e)].join('|');",
    )
    .await;
    assert_eq!(
        out,
        "NotAllowedError|Permission denied|NotAllowedError: Permission denied|true|8|0|[object DOMException]",
        "domex: {out}"
    );
}

#[tokio::test]
async fn fonts_check_against_platform_list_and_rejects_bad_add() {
    let pool = pool();
    let out = eval(
        &pool,
        "var f = document.fonts;\
         var out = [];\
         out.push('status:' + f.status + ':' + f.size);\
         out.push('seg:' + ('Arial' in f));\
         out.push('arial:' + f.check('12px \"Arial\"'));\
         out.push('segoe:' + f.check('12px \"Segoe UI\"'));\
         out.push('bogus:' + f.check('12px \"No Such Font\"'));\
         out.push('generic:' + f.check('12px sans-serif'));\
         out.push('multi:' + f.check('12px \"No Such Font\", Arial'));\
         try { f.add({}); } catch (e) { out.push('add:' + e.name + ':' + (e.message.indexOf(\"parameter 1 is not of type 'FontFace'\") !== -1)); }\
         try { new FontFace('', 'url(x.woff)'); } catch (e) { out.push('ffempty:' + e.name); }\
         try { new FontFace('X', 'nope'); } catch (e) { out.push('ffsrc:' + e.name); }\
         var ff = new FontFace('X', 'url(x.woff)');\
         out.push('ff:' + ff.status + ':' + (typeof ff.load === 'function'));\
         out.push('tag:' + Object.prototype.toString.call(f));\
         out.join('|');",
    )
    .await;
    assert_eq!(
        out,
        "status:loaded:0|seg:false|arial:true|segoe:true|bogus:false|generic:true|multi:true|add:TypeError:true|ffempty:TypeError|ffsrc:TypeError|ff:unloaded:true|tag:[object FontFaceSet]",
        "fonts: {out}"
    );
}

#[tokio::test]
async fn feature_policy_surface_is_chrome_complete() {
    let pool = pool();
    let out = eval(
        &pool,
        "var fp = document.featurePolicy;\
         var feats = fp.features();\
         var allowed = fp.allowedFeatures();\
         [fp.allowsFeature('camera'), fp.allowsFeature('fullscreen'), fp.allowsFeature('idle-detection'),\
          fp.allowsFeature('bogus-feature'), feats.length, allowed.length, feats.indexOf('geolocation') !== -1,\
          typeof document.permissionsPolicy.allowsFeature].join('|');",
    )
    .await;
    let (cam, full, idle, bogus, n_feats, n_allowed, geo, pp) = {
        let parts: Vec<&str> = out.split('|').collect();
        (
            parts[0].to_string(),
            parts[1].to_string(),
            parts[2].to_string(),
            parts[3].to_string(),
            parts[4].parse::<usize>().expect("len"),
            parts[5].parse::<usize>().expect("len"),
            parts[6].to_string(),
            parts[7].to_string(),
        )
    };
    assert_eq!(cam, "true");
    assert_eq!(full, "true");
    assert_eq!(idle, "false");
    assert_eq!(bogus, "false");
    assert!(n_feats >= 35, "features list >= 35: {n_feats}");
    assert!(n_allowed >= 33, "allowed >= 33: {n_allowed}");
    assert_eq!(geo, "true");
    assert_eq!(pp, "function");
}
