(function (P, N) {
    "use strict";

    var W = globalThis;
    var d = W.document;

    var EVQ = {};
    var DLIS = {};
    var WLIS = {};
    var T = 0;
    var heldButtons = 0;
    var lastX = 0, lastY = 0, wheelY = 0;
    var lastClickT = -1e9;
    var clickCount = 0;
    var hidden = false;
    var lastHit = null;

    var KIND_MOVE = 0, KIND_PRESS = 1, KIND_RELEASE = 2, KIND_WHEEL = 3,
        KIND_KEY_DOWN = 4, KIND_KEY_UP = 5, KIND_FOCUS = 6, KIND_BLUR = 7,
        KIND_VISIBILITY = 8;
    var BTN_MASK = [1, 4, 2, 8, 16];

    var DP = Object.getPrototypeOf(d);
    if (!DP || DP === Object.prototype) { DP = d; }
    var WP = Object.getPrototypeOf(W);
    if (!WP || WP === Object.prototype) { WP = W; }

    var P_FN = (typeof P === "function") ? P : null;
    function isTouch() {
        try { return !!(P_FN && P_FN() && P_FN().mobile); } catch (e) { return false; }
    }
    var IS_TOUCH = isTouch();

    function queues() { EVQ = {}; DLIS = {}; WLIS = {}; T = 0; heldButtons = 0; lastX = 0; lastY = 0; wheelY = 0; lastClickT = -1e9; clickCount = 0; hidden = false; lastHit = null; }

    function fire(list, ev) {
        for (var i = 0; i < list.length; i++) {
            try { list[i].call(d, ev); } catch (e) { }
        }
    }

    function publish(type, ev) {
        var q = EVQ[type];
        if (!q) { q = []; EVQ[type] = q; }
        if (q.length < 512) { q.push(ev); }
        var dl = DLIS[type];
        if (dl) { fire(dl, ev); }
        var wl = WLIS[type];
        if (wl) { fire(wl, ev); }
    }

    var EV_TREE = [
        ["Event", null], ["UIEvent", "Event"], ["MouseEvent", "UIEvent"], ["WheelEvent", "MouseEvent"],
        ["PointerEvent", "MouseEvent"], ["DragEvent", "MouseEvent"], ["KeyboardEvent", "UIEvent"],
        ["InputEvent", "UIEvent"], ["CompositionEvent", "UIEvent"], ["FocusEvent", "UIEvent"],
        ["TextEvent", "UIEvent"], ["CustomEvent", "Event"], ["MessageEvent", "Event"],
        ["CloseEvent", "Event"], ["ProgressEvent", "Event"], ["ErrorEvent", "Event"],
        ["PromiseRejectionEvent", "Event"], ["StorageEvent", "Event"], ["PopStateEvent", "Event"],
        ["HashChangeEvent", "Event"], ["PageTransitionEvent", "Event"], ["BeforeUnloadEvent", "Event"],
        ["TransitionEvent", "Event"], ["AnimationEvent", "Event"], ["AnimationPlaybackEvent", "Event"],
        ["SubmitEvent", "Event"], ["GamepadEvent", "Event"], ["MediaEncryptedEvent", "Event"],
        ["MediaStreamTrackEvent", "Event"], ["TrackEvent", "Event"], ["WebGLContextEvent", "Event"],
        ["AudioProcessingEvent", "Event"], ["OfflineAudioCompletionEvent", "Event"],
        ["SpeechSynthesisEvent", "Event"], ["SpeechSynthesisErrorEvent", "SpeechSynthesisEvent"],
        ["ClipboardEvent", "Event"], ["TouchEvent", "UIEvent"], ["FontFaceSetLoadEvent", "Event"],
        ["Touch", null], ["TouchList", null]
    ];
    var EV_DEFAULTS = {
        bubbles: false, cancelable: false, composed: false, detail: null, view: null
    };
    function initEventObj(ev, type, opts) {
        ev.type = type;
        ev.isTrusted = false;
        ev.target = null;
        ev.currentTarget = null;
        ev.timeStamp = T;
        ev.defaultPrevented = false;
        ev.eventPhase = 0;
        ev.returnValue = true;
        for (var dk in EV_DEFAULTS) {
            if (!Object.prototype.hasOwnProperty.call(ev, dk)) { ev[dk] = EV_DEFAULTS[dk]; }
        }
        if (opts) {
            for (var k in opts) {
                if (k !== "isTrusted" && Object.prototype.hasOwnProperty.call(opts, k)) { ev[k] = opts[k]; }
            }
        }
    }
    var EV_PROTO = {
        preventDefault: function () { },
        stopPropagation: function () { },
        stopImmediatePropagation: function () { },
        initEvent: function (type, bubbles, cancelable) {
            this.type = type; this.bubbles = !!bubbles; this.cancelable = !!cancelable;
        },
        initCustomEvent: function (type, bubbles, cancelable, detail) {
            this.initEvent(type, bubbles, cancelable); this.detail = detail;
        },
        initUIEvent: function (type, bubbles, cancelable, view, detail) {
            this.initEvent(type, bubbles, cancelable); this.view = view; this.detail = detail;
        },
        initMouseEvent: function (type, bubbles, cancelable, view, detail, cx, cy, cx2, cy2) {
            this.initUIEvent(type, bubbles, cancelable, view, detail);
            this.clientX = cx; this.clientY = cy; this.screenX = cx2; this.screenY = cy2;
        },
        composedPath: function () { return []; }
    };
    var EV_CLASSES = {};
    for (var ei = 0; ei < EV_TREE.length; ei++) {
        (function (name, parentName) {
            var C = function (type, opts) {
                if (!(this instanceof C)) { return new C(type, opts); }
                initEventObj(this, type, opts);
                if (name === "CustomEvent" && opts && "detail" in opts === false) { this.detail = null; }
                if (name === "Touch" && opts) {
                    this.identifier = 0; this.clientX = 0; this.clientY = 0; this.screenX = 0;
                    this.screenY = 0; this.pageX = 0; this.pageY = 0; this.radiusX = 11.5;
                    this.radiusY = 11.5; this.rotationAngle = 0; this.force = 1;
                    initEventObj(this, type, opts);
                }
            };
            C.prototype = Object.create(parentName ? EV_CLASSES[parentName].prototype : Object.prototype);
            C.prototype.constructor = C;
            if (name !== "Touch" && name !== "TouchList") {
                for (var pm in EV_PROTO) { C.prototype[pm] = EV_PROTO[pm]; }
            }
            EV_CLASSES[name] = C;
            W[name] = C;
        })(EV_TREE[ei][0], EV_TREE[ei][1]);
    }
    EV_CLASSES.Event.prototype.preventDefault = EV_PROTO.preventDefault;

    var TYPE_CLASS = {
        click: "MouseEvent", dblclick: "MouseEvent", mousedown: "MouseEvent", mouseup: "MouseEvent",
        mousemove: "MouseEvent", mouseover: "MouseEvent", mouseout: "MouseEvent", mouseenter: "MouseEvent",
        mouseleave: "MouseEvent", contextmenu: "MouseEvent", dragstart: "DragEvent", drag: "DragEvent",
        dragend: "DragEvent", dragover: "DragEvent", dragenter: "DragEvent", dragleave: "DragEvent",
        drop: "DragEvent", pointerdown: "PointerEvent", pointerup: "PointerEvent", pointermove: "PointerEvent",
        pointerover: "PointerEvent", pointerout: "PointerEvent", pointerenter: "PointerEvent",
        pointerleave: "PointerEvent", pointercancel: "PointerEvent", wheel: "WheelEvent",
        keydown: "KeyboardEvent", keyup: "KeyboardEvent", keypress: "KeyboardEvent",
        input: "InputEvent", compositionstart: "CompositionEvent", compositionupdate: "CompositionEvent",
        compositionend: "CompositionEvent", focus: "FocusEvent", blur: "FocusEvent", focusin: "FocusEvent",
        focusout: "FocusEvent", touchstart: "TouchEvent", touchmove: "TouchEvent", touchend: "TouchEvent",
        touchcancel: "TouchEvent", message: "MessageEvent", error: "ErrorEvent", close: "CloseEvent",
        loadstart: "ProgressEvent", progress: "ProgressEvent", loadend: "ProgressEvent", abort: "ProgressEvent",
        timeout: "ProgressEvent", storage: "StorageEvent", popstate: "PopStateEvent",
        hashchange: "HashChangeEvent", pageshow: "PageTransitionEvent", pagehide: "PageTransitionEvent",
        submit: "SubmitEvent", gamepadconnected: "GamepadEvent", gamepaddisconnected: "GamepadEvent",
        transitionend: "TransitionEvent", animationend: "AnimationEvent", animationstart: "AnimationEvent",
        animationiteration: "AnimationEvent", webglcontextlost: "WebGLContextEvent",
        webglcontextrestored: "WebGLContextEvent", encrypted: "MediaEncryptedEvent",
        scroll: "Event", resize: "Event", load: "Event", beforeunload: "BeforeUnloadEvent",
        visibilitychange: "Event", readystatechange: "Event", DOMContentLoaded: "Event",
        disconnect: "Event", connect: "Event", languagechange: "Event", offline: "Event", online: "Event",
        orientationchange: "Event", copy: "ClipboardEvent", cut: "ClipboardEvent", paste: "ClipboardEvent"
    };

    function makeEvent(type, props) {
        var cls = EV_CLASSES[TYPE_CLASS[type] || "Event"];
        var ev = new cls(type, props);
        ev.target = (props && props.target) || d;
        ev.currentTarget = (props && props.currentTarget) || d;
        ev.view = W;
        if (!props || !("bubbles" in props)) { ev.bubbles = true; }
        if (!props || !("cancelable" in props)) { ev.cancelable = true; }
        if (!props || !("composed" in props)) { ev.composed = true; }
        if (!props || !("eventPhase" in props)) { ev.eventPhase = 2; }
        Object.defineProperty(ev, "isTrusted", { value: true, writable: false, configurable: false });
        return ev;
    }

    function mouseProps(x, y, buttons, button, detail) {
        var pt = IS_TOUCH ? "touch" : "mouse";
        return {
            clientX: x, clientY: y,
            screenX: x, screenY: y,
            pageX: x, pageY: y + wheelY,
            offsetX: x, offsetY: y,
            x: x, y: y,
            movementX: IS_TOUCH ? 0 : x - lastX, movementY: IS_TOUCH ? 0 : y - lastY,
            buttons: buttons, button: button,
            detail: detail,
            ctrlKey: false, shiftKey: false, altKey: false, metaKey: false,
            relatedTarget: null,
            pointerId: 1, pointerType: pt, isPrimary: true,
            pressure: IS_TOUCH ? (buttons === 0 ? 0 : 1) : (buttons === 0 ? 0 : 0.5),
            width: IS_TOUCH ? 23 : 1, height: IS_TOUCH ? 23 : 1, twist: 0, tiltX: 0, tiltY: 0
        };
    }
    function touchProps(x, y, target) {
        var tp = { identifier: 1, clientX: x, clientY: y, screenX: x, screenY: y, pageX: x, pageY: y + wheelY, radiusX: 11.5, radiusY: 11.5, rotationAngle: 0, force: 1, target: target };
        return { touches: [], targetTouches: [], changedTouches: [tp] };
    }

    function keyProps(code, shift) {
        var c = code | 0;
        var key, kcode;
        if (c === 8) { key = "Backspace"; kcode = 8; }
        else if (c === 9) { key = "Tab"; kcode = 9; }
        else if (c === 13) { key = "Enter"; kcode = 13; }
        else if (c === 27) { key = "Escape"; kcode = 27; }
        else if (c === 32) { key = " "; kcode = 32; }
        else { key = String.fromCharCode(c); kcode = c >= 32 && c < 127 ? key.toUpperCase().charCodeAt(0) : 0; }
        var isLetter = (c >= 65 && c <= 90) || (c >= 97 && c <= 122);
        return {
            key: key,
            code: isLetter ? "Key" + key.toUpperCase() : (kcode >= 48 && kcode <= 57 ? "Digit" + key : (c === 8 ? "Backspace" : "")),
            keyCode: kcode,
            which: kcode,
            charCode: 0,
            shiftKey: shift === 1,
            ctrlKey: false, altKey: false, metaKey: false,
            repeat: false,
            locale: "",
            detail: 0
        };
    }

    function fixProps(pr, tg) { pr.target = tg; pr.currentTarget = tg; return pr; }

    function hitAt(x, y) {
        try { if (typeof d.elementFromPoint === "function") { var t = d.elementFromPoint(x, y); if (t && t !== d) { return t; } } } catch (e) { }
        return null;
    }

    function enterTarget(t, x, y) {
        if (lastHit && lastHit !== t) {
            var lv = mouseProps(x, y, heldButtons, 0, 0);
            fixProps(lv, lastHit);
            publish("pointerout", makeEvent("pointerout", lv));
            publish("mouseout", makeEvent("mouseout", lv));
            publish("pointerleave", makeEvent("pointerleave", lv));
            publish("mouseleave", makeEvent("mouseleave", lv));
        }
        var en = fixProps(mouseProps(x, y, heldButtons, 0, 0), t);
        publish("pointerover", makeEvent("pointerover", en));
        publish("mouseover", makeEvent("mouseover", en));
        publish("pointerenter", makeEvent("pointerenter", en));
        publish("mouseenter", makeEvent("mouseenter", en));
        lastHit = t;
    }

    function feed(x, y, dt, kind, arg) {
        T += dt;
        x = x | 0; y = y | 0; kind = kind | 0; arg = arg | 0;
        IS_TOUCH = isTouch();
        if (kind === KIND_MOVE) {
            var mp = mouseProps(x, y, heldButtons, 0, 0);
            publish("pointermove", makeEvent("pointermove", mp));
            if (IS_TOUCH) {
                if (heldButtons !== 0) {
                    var ttp = touchProps(x, y, lastHit || d);
                    ttp.touches = [ttp.changedTouches[0]];
                    ttp.targetTouches = [ttp.changedTouches[0]];
                    publish("touchmove", makeEvent("touchmove", ttp));
                }
            } else {
                publish("mousemove", makeEvent("mousemove", mp));
            }
            lastX = x; lastY = y;
        } else if (kind === KIND_PRESS) {
            var t = hitAt(x, y);
            if (t && t !== lastHit) { enterTarget(t, x, y); }
            var target = lastHit || d;
            var mask = BTN_MASK[arg] || 1;
            heldButtons |= mask;
            var dp = fixProps(mouseProps(x, y, heldButtons, arg, 1), target);
            publish("pointerdown", makeEvent("pointerdown", dp));
            if (IS_TOUCH) {
                var tps = touchProps(x, y, target);
                tps.touches = [tps.changedTouches[0]];
                tps.targetTouches = [tps.changedTouches[0]];
                publish("touchstart", makeEvent("touchstart", tps));
            } else {
                publish("mousedown", makeEvent("mousedown", dp));
            }
        } else if (kind === KIND_RELEASE) {
            var target = lastHit || d;
            var maskr = BTN_MASK[arg] || 1;
            heldButtons &= ~maskr;
            var up = fixProps(mouseProps(x, y, heldButtons, arg, 1), target);
            publish("pointerup", makeEvent("pointerup", up));
            if (IS_TOUCH) {
                var tpe = touchProps(x, y, target);
                publish("touchend", makeEvent("touchend", tpe));
            } else {
                publish("mouseup", makeEvent("mouseup", up));
            }
            if (T - lastClickT < 500) { clickCount++; } else { clickCount = 1; }
            lastClickT = T;
            var cp = fixProps(mouseProps(x, y, heldButtons, arg, clickCount), target);
            if (arg === 2) {
                publish("contextmenu", makeEvent("contextmenu", cp));
            } else {
                if (IS_TOUCH) {
                    var mm = fixProps(mouseProps(x, y, heldButtons, 0, 0), target);
                    mm.movementX = 0; mm.movementY = 0;
                    publish("mousemove", makeEvent("mousemove", mm));
                    publish("mousedown", makeEvent("mousedown", cp));
                    publish("mouseup", makeEvent("mouseup", cp));
                }
                publish("click", makeEvent("click", cp));
                if (clickCount >= 2) {
                    var dc = fixProps(mouseProps(x, y, heldButtons, arg, clickCount), target);
                    publish("dblclick", makeEvent("dblclick", dc));
                }
            }
        } else if (kind === KIND_WHEEL) {
            var delta = y - wheelY;
            wheelY = y;
            var wp = {
                deltaX: 0, deltaY: delta, deltaZ: 0,
                deltaMode: 0,
                clientX: lastX, clientY: lastY,
                screenX: lastX, screenY: lastY,
                pageX: lastX, pageY: y,
                x: lastX, y: lastY,
                buttons: heldButtons, button: 0, detail: 0,
                ctrlKey: false, shiftKey: false, altKey: false, metaKey: false
            };
            publish("wheel", makeEvent("wheel", wp));
            publish("scroll", makeEvent("scroll", { detail: delta }));
        } else if (kind === KIND_KEY_DOWN) {
            publish("keydown", makeEvent("keydown", keyProps(x, arg)));
        } else if (kind === KIND_KEY_UP) {
            publish("keyup", makeEvent("keyup", keyProps(x, arg)));
        } else if (kind === KIND_FOCUS) {
            publish("focus", makeEvent("focus", { target: W, bubbles: false }));
        } else if (kind === KIND_BLUR) {
            publish("blur", makeEvent("blur", { target: W, bubbles: false }));
        } else if (kind === KIND_VISIBILITY) {
            hidden = arg === 1;
            if (hidden) {
                publish("blur", makeEvent("blur", { target: W, bubbles: false }));
                publish("visibilitychange", makeEvent("visibilitychange", {}));
            } else {
                publish("visibilitychange", makeEvent("visibilitychange", {}));
                publish("focus", makeEvent("focus", { target: W, bubbles: false }));
            }
        }
    }

    function reset() {
        queues();
    }

    function register(store, type, fn) {
        if (typeof fn !== "function") { return; }
        var list = store[type];
        if (!list) { list = []; store[type] = list; }
        if (list.length < 128) { list.push(fn); }
        var q = EVQ[type];
        if (q && q.length) {
            for (var i = 0; i < q.length; i++) {
                try { fn.call(d, q[i]); } catch (e) { }
            }
        }
    }

    DP.addEventListener = function (type, fn) { register(DLIS, type, fn); };
    DP.removeEventListener = function (type, fn) {
        var list = DLIS[type];
        if (!list) { return; }
        for (var i = 0; i < list.length; i++) {
            if (list[i] === fn) { list.splice(i, 1); return; }
        }
    };
    WP.addEventListener = function (type, fn) { register(WLIS, type, fn); };
    WP.removeEventListener = function (type, fn) {
        var list = WLIS[type];
        if (!list) { return; }
        for (var i = 0; i < list.length; i++) {
            if (list[i] === fn) { list.splice(i, 1); return; }
        }
    };

    Object.defineProperty(DP, "hidden", { get: function () { return hidden; }, configurable: true });
    Object.defineProperty(DP, "visibilityState", { get: function () { return hidden ? "hidden" : "visible"; }, configurable: true });
    Object.defineProperty(DP, "hasFocus", { value: function () { return !hidden; }, configurable: true });

    function fireBoth(type, ev) {
        var dl = DLIS[type];
        if (dl) { fire(dl, ev); }
        var wl = WLIS[type];
        if (wl) { fire(wl, ev); }
    }

    W.__silo_domready = function () {
        fireBoth("DOMContentLoaded", makeEvent("DOMContentLoaded", { target: d, currentTarget: d, bubbles: false, cancelable: false }));
        fireBoth("readystatechange", makeEvent("readystatechange", { target: d, currentTarget: d }));
    };

    W.__silo_pageload = function () {
        fireBoth("readystatechange", makeEvent("readystatechange", { target: d, currentTarget: d }));
        fireBoth("load", makeEvent("load", { target: W, currentTarget: W }));
        var wps = WLIS["pageshow"];
        if (wps) { fire(wps, makeEvent("pageshow", { target: W, currentTarget: W, persisted: false })); }
    };

    function makeStorage() {
        var keys = [];
        var vals = {};
        function find(k) {
            for (var i = 0; i < keys.length; i++) { if (keys[i] === k) { return i; } }
            return -1;
        }
        return {
            getItem: function (k) { k = String(k); return Object.prototype.hasOwnProperty.call(vals, k) ? vals[k] : null; },
            setItem: function (k, v) {
                k = String(k); v = String(v);
                if (find(k) < 0) { keys.push(k); }
                vals[k] = v;
            },
            removeItem: function (k) {
                k = String(k);
                var i = find(k);
                if (i >= 0) { keys.splice(i, 1); }
                delete vals[k];
            },
            clear: function () { keys.length = 0; vals = {}; },
            key: function (i) { return i >= 0 && i < keys.length ? keys[i] : null; },
            get length() { return keys.length; }
        };
    }
    try { Object.defineProperty(W, "localStorage", { value: makeStorage(), configurable: true, writable: false }); } catch (e) { }
    try { Object.defineProperty(W, "sessionStorage", { value: makeStorage(), configurable: true, writable: false }); } catch (e) { }
    Object.defineProperty(W, "crossOriginIsolated", { value: false, configurable: false, writable: false });
    Object.defineProperty(W, "isSecureContext", { value: true, configurable: false, writable: false });

    W.window = W;
    W.self = W;
    W.top = W;
    W.parent = W;
    W.frames = W;
    W.name = "";
    W.closed = false;
    W.opener = null;
    W.origin = (W.location && W.location.origin) || "null";
    var B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    W.history = { length: 1, state: null, scrollRestoration: "auto", pushState: function () { }, replaceState: function () { }, back: function () { }, forward: function () { }, go: function () { } };
    W.atob = function (s) {
        var out = [], bits = 0, acc2 = 0;
        for (var i = 0; i < s.length; i++) {
            var v = B64.indexOf(s.charAt(i));
            if (v < 0) { continue; }
            acc2 = (acc2 << 6) | v;
            bits += 6;
            if (bits >= 8) {
                bits -= 8;
                out.push((acc2 >> bits) & 255);
            }
        }
        var str = "";
        for (var j = 0; j < out.length; j++) { str += String.fromCharCode(out[j]); }
        return str;
    };
    W.btoa = function (s) {
        var out = "", i;
        for (i = 0; i + 2 < s.length; i += 3) {
            var n = (s.charCodeAt(i) << 16) | (s.charCodeAt(i + 1) << 8) | s.charCodeAt(i + 2);
            out += B64.charAt(n >> 18) + B64.charAt((n >> 12) & 63) + B64.charAt((n >> 6) & 63) + B64.charAt(n & 63);
        }
        var tail = s.length - i;
        if (tail === 1) {
            var k = s.charCodeAt(i) << 16;
            out += B64.charAt(k >> 18) + B64.charAt((k >> 12) & 63) + B64.charAt((k >> 6) & 63) + "==";
        } else if (tail === 2) {
            var m = (s.charCodeAt(i) << 16) | (s.charCodeAt(i + 1) << 8);
            out += B64.charAt(m >> 18) + B64.charAt((m >> 12) & 63) + B64.charAt((m >> 6) & 63) + B64.charAt(m & 63);
        }
        return out;
    };

    W.fetch = function () {
        return Promise.resolve({
            ok: true,
            status: 204,
            statusText: "No Content",
            headers: {},
            url: "",
            redirected: false,
            type: "basic",
            json: function () { return Promise.resolve({}); },
            text: function () { return Promise.resolve(""); },
            arrayBuffer: function () { return Promise.resolve(new ArrayBuffer(0)); },
            clone: function () { return W.fetch(); },
            blob: function () { return Promise.resolve({ size: 0, type: "" }); }
        });
    };

    function pluginEntry(name, filename, desc, ver) {
        var e = {
            name: name, filename: filename, description: desc, version: ver,
            length: 1, item: function (i) { return i === 0 ? e : null; },
            named: function (n) { return n === name ? e : null; }
        };
        e[0] = { type: "application/pdf", suffixes: "pdf", description: desc, enabledPlugin: e };
        return e;
    }

    var PDFS = [
        pluginEntry("PDF Viewer", "internal-pdf-viewer", "Portable Document Format", "2.0"),
        pluginEntry("Chrome PDF Viewer", "internal-pdf-viewer", "Portable Document Format", "2.0"),
        pluginEntry("Chromium PDF Viewer", "internal-pdf-viewer", "Portable Document Format", "2.0"),
        pluginEntry("Microsoft Edge PDF Viewer", "internal-pdf-viewer", "Portable Document Format", "2.0"),
        pluginEntry("WebKit built-in PDF", "internal-pdf-viewer", "Portable Document Format", "2.0")
    ];

    var plugins = {
        length: 5,
        item: function (i) { return i >= 0 && i < 5 ? PDFS[i] : null; },
        named: function (n) {
            for (var i = 0; i < 5; i++) { if (PDFS[i].name === n) { return PDFS[i]; } }
            return null;
        },
        refresh: function () { }
    };
    for (var pi = 0; pi < 5; pi++) { plugins[pi] = PDFS[pi]; }

    var mimeTypes = {
        length: 2,
        item: function (i) { return i >= 0 && i < 2 ? this[i] : null; },
        named: function (n) { return this[n] || null; }
    };
    mimeTypes[0] = { type: "application/pdf", suffixes: "pdf", description: "Portable Document Format", enabledPlugin: PDFS[0] };
    mimeTypes[1] = { type: "text/pdf", suffixes: "pdf", description: "Portable Document Format", enabledPlugin: PDFS[0] };
    mimeTypes["application/pdf"] = mimeTypes[0];
    mimeTypes["text/pdf"] = mimeTypes[1];

    var orientation = { onchange: null };
    Object.defineProperty(orientation, "type", {
        get: function () { var s = profScreen(); return s.w >= s.h ? "landscape-primary" : "portrait-primary"; },
        enumerable: true, configurable: true
    });
    Object.defineProperty(orientation, "angle", {
        get: function () { var s = profScreen(); return s.w >= s.h ? 0 : 90; },
        enumerable: true, configurable: true
    });

    var uaData = {
        get brands() { return parseBrands((prof() || {}).secChUa); },
        get mobile() { return profMobile(); },
        get platform() { return profPlatform(); },
        getHighEntropyValues: function (hints) {
            var out = {};
            hints = hints || [];
            var self = this;
            var full = realFullVersion();
            for (var i = 0; i < hints.length; i++) {
                var h = hints[i];
                if (h === "architecture") { out.architecture = "x86"; }
                else if (h === "bitness") { out.bitness = "64"; }
                else if (h === "model") { out.model = ""; }
                else if (h === "platformVersion") { out.platformVersion = platformVersionOf(profUa(), profPlatform()); }
                else if (h === "uaFullVersion") { out.uaFullVersion = full; }
                else if (h === "fullVersionList") { out.fullVersionList = self.brands.map(function (b) { return { brand: b.brand, version: isRealBrand(b.brand) ? full : b.version }; }); }
                else if (h === "wow64") { out.wow64 = false; }
            }
            return Promise.resolve(out);
        },
        toJSON: function () { return { brands: this.brands, mobile: this.mobile, platform: this.platform }; }
    };

    function prof() { try { return (typeof P === "function" && P()) || null; } catch (e) { return null; } }
    function profUa() { var p = prof(); return (p && p.ua) || ""; }
    function profMobile() { var p = prof(); return !!(p && p.mobile); }
    function profScreen() { var p = prof(); return p ? { w: p.screenW || 1920, h: p.screenH || 1080 } : { w: 1920, h: 1080 }; }
    function profPlatform() { var p = prof(); return (p && p.platform) || "Windows"; }

    function parseBrands(sec) {
        var out = [];
        if (typeof sec !== "string" || !sec) { return out; }
        var re = /"([^"]*)";v="([^"]*)"/g, m;
        while ((m = re.exec(sec)) !== null) { out.push({ brand: m[1], version: m[2] }); }
        return out;
    }

    function fullVersionOf(ua) {
        var m = /(?:Chrome|Edg|Firefox|Version)\/([0-9][0-9.]*)/.exec(ua);
        return m ? m[1] : "0.0.0.0";
    }

    function seedNum() {
        var p = prof();
        var s = (p && typeof p.seed === "number") ? (p.seed | 0) : 0x2545F491;
        s = s ^ (s >>> 15); s = Math.imul(s, 0x85EBCA6B); s = s ^ (s >>> 13);
        return s >>> 0;
    }

    function fullVersionFromSeed(ua) {
        var major = fullVersionOf(ua).split(".")[0] || "0";
        var s = seedNum();
        var b = (s >>> 8) % 10000;
        var patch = (s >>> 20) % 400;
        return major + ".0." + b + "." + patch;
    }

    function realFullVersion() {
        try {
            var v = N && typeof N.uaFullVersion === "function" ? N.uaFullVersion() : "";
            if (typeof v === "string" && v) { return v; }
        } catch (e) { }
        return fullVersionFromSeed(profUa());
    }

    function isRealBrand(name) {
        return name === "Chromium" || name === "Google Chrome" || name === "Microsoft Edge";
    }

    function platformVersionOf(ua, plat) {
        if (plat === "Windows") {
            if (/Windows NT 10\.0/.test(ua)) { return "15.0.0"; }
            if (/Windows NT 6\.3/.test(ua)) { return "6.3.0"; }
            if (/Windows NT 6\.1/.test(ua)) { return "0.1.0"; }
            return "1.0.0";
        }
        if (plat === "macOS") {
            var m = /Mac OS X 10_([0-9]+)_([0-9]+)/.exec(ua);
        if (m) { var maj = 10 + Number(m[1]); return maj + "." + (m[2] || 0) + ".0"; }
            return "10.15.7";
        }
        if (plat === "Linux") {
            var s = seedNum();
            return "6." + ((s >>> 4) % 9) + "." + ((s >>> 12) % 60);
        }
        return "";
    }


    var chromeObj = {
        runtime: {
            id: undefined,
            getURL: function (p) { return "chrome-extension://invalid/" + String(p); },
            getManifest: function () { return { }; },
            sendMessage: function () { },
            connect: function () { return { onMessage: { addListener: function () { } }, postMessage: function () { } }; }
        },
        app: {
            isInstalled: false,
            getDetails: function () { return null; },
            getIsInstalled: function () { return false; },
            InstallState: { DISABLED: 0, INSTALLED: 1, NOT_INSTALLED: 2 },
            RunningState: { CANNOT_RUN: 0, READY_TO_RUN: 1, RUNNING: 2 }
        },
        csi: function () { return { }; },
        loadTimes: function () { return { requestTime: 0, startLoadTime: 0, commitLoadTime: 0, finishDocumentLoadTime: 0, finishLoadTime: 0, firstPaintTime: 0, firstPaintAfterLoadTime: 0, navigationType: "Other", wasFetchedViaSpdy: false, wasNpnNegotiated: true, wasAlternateProtocolAvailable: false, connectionInfo: "h2" }; }
    };

    W.__silo_stubpack = {
        plugins: plugins,
        mimeTypes: mimeTypes,
        orientation: orientation,
        uaData: uaData,
        chrome: chromeObj
    };

    function NotificationCtor(title, opts) {
        this.title = title;
        this.body = opts && opts.body || "";
        this.tag = opts && opts.tag || "";
        this.onclick = null;
        this.onshow = null;
        this.onerror = null;
        this.onclose = null;
    }
    NotificationCtor.permission = "default";
    NotificationCtor.maxActions = 2;
    NotificationCtor.requestPermission = function (cb) {
        var p = Promise.resolve("default");
        if (typeof cb === "function") { p.then(cb); }
        return p;
    };
    NotificationCtor.prototype.close = function () { };
    NotificationCtor.prototype.show = function () { };
    W.Notification = NotificationCtor;

    function WorkerCtor(src) {
        this.onmessage = null;
        this.onerror = null;
        this._src = src;
    }
    WorkerCtor.prototype.postMessage = function () { };
    WorkerCtor.prototype.terminate = function () { };
    WorkerCtor.prototype.addEventListener = function () { };
    WorkerCtor.prototype.removeEventListener = function () { };
    W.Worker = WorkerCtor;

    function lcg(seed) {
        var s = seed | 0;
        return function () {
            s = (Math.imul(s, 1664525) + 1013904223) | 0;
            return (s >>> 8) / 16777216;
        };
    }

    function fillChannel(arr, seed) {
        var next = lcg(seed);
        for (var i = 0; i < arr.length; i++) {
            arr[i] = (next() - 0.5) * 1.0;
        }
        return arr;
    }

    function audioCompressorNodes(reduction) {
        var out = {
            threshold: { value: -24 }, knee: { value: 30 }, ratio: { value: 12 },
            attack: { value: 0.003 }, release: { value: 0.25 },
            reduction: reduction,
            context: null,
            connect: function () { }, disconnect: function () { }
        };
        Object.setPrototypeOf(out, W.DynamicsCompressorNode.prototype);
        return out;
    }

    function audioBufferNodes(self, ch, len, rate) {
        var chans = [];
        for (var c = 0; c < ch; c++) { chans.push(fillChannel(new Float32Array(len), self._seed + c)); }
        var out = {
            numberOfChannels: ch, length: len, sampleRate: rate, duration: len / rate,
            getChannelData: function (c) { return chans[c] || new Float32Array(len); }
        };
        Object.setPrototypeOf(out, W.AudioBuffer ? W.AudioBuffer.prototype : Object.prototype);
        return out;
    }

    var AUDIO_NODES = ["GainNode", "OscillatorNode", "DynamicsCompressorNode", "AnalyserNode",
        "BiquadFilterNode", "DelayNode", "PannerNode", "StereoPannerNode", "WaveShaperNode",
        "ConvolverNode", "ConstantSourceNode", "AudioBufferSourceNode", "ChannelMergerNode",
        "ChannelSplitterNode", "ScriptProcessorNode"];
    for (var an = 0; an < AUDIO_NODES.length; an++) {
        (function (name) {
            var C = function () { this.context = null; };
            C.prototype.connect = function () { };
            C.prototype.disconnect = function () { };
            W[name] = C;
        })(AUDIO_NODES[an]);
    }

    function gainNode(ctx) {
        var g = { gain: { value: 1 }, connect: function () { }, disconnect: function () { } };
        if (ctx) { g.context = ctx; }
        Object.setPrototypeOf(g, W.GainNode.prototype);
        return g;
    }

    function AudioContextCtor() {
        this.sampleRate = 44100;
        this.state = "running";
        this.currentTime = 0;
        this.baseLatency = 0.005333333333333333;
        this.destination = { channelCount: 2, maxChannelCount: 2, channelCountMode: "explicit", channelInterpretation: "speakers" };
        this.listener = { forwardX: { value: 0 }, forwardY: { value: 0 }, forwardZ: { value: -1 }, positionX: { value: 0 }, positionY: { value: 0 }, positionZ: { value: 0 } };
        this._seed = seedNum() | 1;
        var self = this;
        var reduction = N.audioFp();
        this.createDynamicsCompressor = function () { return audioCompressorNodes(reduction); };
        this.createBuffer = function (ch, len, rate) { return audioBufferNodes(self, ch, len, rate); };
        this.createGain = function () { return gainNode(self); };
        this.createOscillator = function () {
            var o = { frequency: { value: 440 }, type: "sine", connect: function () { }, start: function () { }, stop: function () { }, disconnect: function () { } };
            Object.setPrototypeOf(o, W.OscillatorNode.prototype);
            return o;
        };
        this.createAnalyser = function () {
            var a = { fftSize: 2048, frequencyBinCount: 1024, getFloatFrequencyData: function () { }, getByteFrequencyData: function () { }, connect: function () { }, disconnect: function () { } };
            Object.setPrototypeOf(a, W.AnalyserNode.prototype);
            return a;
        };
        this.decodeAudioData = function () { return Promise.resolve(self.createBuffer(2, 128, 44100)); };
        this.resume = function () { return Promise.resolve(); };
        this.suspend = function () { return Promise.resolve(); };
        this.close = function () { return Promise.resolve(); };
    }
    W.AudioContext = AudioContextCtor;
    W.webkitAudioContext = AudioContextCtor;

    function OfflineAudioContextCtor(ch, len, rate) {
        this.numberOfChannels = ch;
        this.length = len;
        this.sampleRate = rate;
        this.destination = { channelCount: ch, maxChannelCount: ch, channelCountMode: "explicit", channelInterpretation: "speakers" };
        this._seed = seedNum() | 1;
        var self = this;
        var reduction = N.audioFp();
        this.createDynamicsCompressor = function () { return audioCompressorNodes(reduction); };
        this.createBuffer = function (c, l, r) { return audioBufferNodes(self, c, l, r); };
        this.createGain = function () { return gainNode(self); };
        this.startRendering = function () {
            var buf = audioBufferNodes(self, self.numberOfChannels, self.length, self.sampleRate);
            if (typeof self.oncomplete === "function") { try { self.oncomplete({ renderedBuffer: buf }); } catch (e) { } }
            return Promise.resolve(buf);
        };
        this.resume = function () { return Promise.resolve(); };
    }
    W.OfflineAudioContext = OfflineAudioContextCtor;
    W.webkitOfflineAudioContext = OfflineAudioContextCtor;

    function URLSearchParamsCtor(init) {
        this._pairs = [];
        if (typeof init === "string") {
            if (init.charAt(0) === "?") { init = init.slice(1); }
            var parts = init.split("&");
            for (var i = 0; i < parts.length; i++) {
                if (!parts[i]) { continue; }
                var kv = parts[i].split("=");
                this._pairs.push([decodeURIComponent(kv[0] || ""), decodeURIComponent(kv[1] || "")]);
            }
        } else if (init && typeof init.forEach === "function") {
            var self = this;
            init.forEach(function (v, k) { self._pairs.push([String(k), String(v)]); });
        }
    }
    function pairMethods(C) {
        C.prototype.get = function (k) {
            for (var i = 0; i < this._pairs.length; i++) { if (this._pairs[i][0] === String(k)) { return this._pairs[i][1]; } }
            return null;
        };
        C.prototype.getAll = function (k) {
            var out = [];
            for (var i = 0; i < this._pairs.length; i++) { if (this._pairs[i][0] === String(k)) { out.push(this._pairs[i][1]); } }
            return out;
        };
        C.prototype.has = function (k) {
            for (var i = 0; i < this._pairs.length; i++) { if (this._pairs[i][0] === String(k)) { return true; } }
            return false;
        };
        C.prototype.delete = function (k) {
            var out = [];
            for (var i = 0; i < this._pairs.length; i++) { if (this._pairs[i][0] !== String(k)) { out.push(this._pairs[i]); } }
            this._pairs = out;
        };
        C.prototype.forEach = function (fn) {
            for (var i = 0; i < this._pairs.length; i++) { fn(this._pairs[i][1], this._pairs[i][0]); }
        };
    }
    pairMethods(URLSearchParamsCtor);
    URLSearchParamsCtor.prototype.append = function (k, v) { this._pairs.push([String(k), String(v)]); };
    URLSearchParamsCtor.prototype.set = function (k, v) {
        for (var i = 0; i < this._pairs.length; i++) {
            if (this._pairs[i][0] === String(k)) { this._pairs[i][1] = String(v); return; }
        }
        this.append(k, v);
    };
    URLSearchParamsCtor.prototype.toString = function () {
        var out = [];
        for (var i = 0; i < this._pairs.length; i++) {
            out.push(encodeURIComponent(this._pairs[i][0]) + "=" + encodeURIComponent(this._pairs[i][1]));
        }
        return out.join("&");
    };
        W.URLSearchParams = URLSearchParamsCtor;

    function URLCtor(url, base) {
        if (!(this instanceof URLCtor)) { return new URLCtor(url, base); }
        url = String(url);
        if (base !== undefined && !/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(url)) {
            var b = new URLCtor(base);
            var dir = b.pathname.slice(0, b.pathname.lastIndexOf("/") + 1);
            url = b.protocol + "//" + b.host + dir + url;
        }
        var m = /^([a-zA-Z][a-zA-Z0-9+.-]*:)(?:\/\/([^\/?#]*))?([^?#]*)(\?[^#]*)?(#.*)?/.exec(url);
        if (!m) { m = [":", ":", "", "", "", ""]; }
        this._u = m;
        this.protocol = m[1] || ":";
        var auth = m[2] || "";
        var at = auth.lastIndexOf("@");
        var hostpart = at >= 0 ? auth.slice(at + 1) : auth;
        var colon = hostpart.lastIndexOf(":");
        this.hostname = colon >= 0 ? hostpart.slice(0, colon) : hostpart;
        this.port = colon >= 0 ? hostpart.slice(colon + 1) : "";
        this.host = hostpart;
        this.pathname = m[3] || "/";
        this.search = m[4] || "";
        this.hash = m[5] || "";
        this.href = this.protocol + (auth ? "//" + auth : "") + this.pathname + this.search + this.hash;
        this.origin = this.protocol + "//" + this.host;
        var self = this;
        this.searchParams = new URLSearchParamsCtor(this.search);
        Object.defineProperty(this, "search", {
            get: function () { return self.searchParams.toString() ? "?" + self.searchParams.toString() : ""; },
            set: function (v) { self.searchParams = new URLSearchParamsCtor(String(v)); },
            configurable: true, enumerable: true
        });
    }
    URLCtor.prototype.toString = function () { return this.protocol + (this._u[2] ? "//" + this._u[2] : "") + this.pathname + "?" + this.searchParams.toString() + this.hash; };
    W.URL = URLCtor;

    function TextEncoderCtor() { }
    TextEncoderCtor.prototype.encode = function (s) {
        s = String(s);
        var out = [];
        for (var i = 0; i < s.length; i++) {
            var code = s.charCodeAt(i);
            if (code < 0x80) { out.push(code); }
            else if (code < 0x800) { out.push(0xC0 | (code >> 6), 0x80 | (code & 63)); }
            else if (code >= 0xD800 && code <= 0xDBFF && i + 1 < s.length) {
                var lo = s.charCodeAt(i + 1);
                if (lo >= 0xDC00 && lo <= 0xDFFF) {
                    code = 0x10000 + ((code - 0xD800) << 10) + (lo - 0xDC00);
                    out.push(0xF0 | (code >> 18), 0x80 | ((code >> 12) & 63), 0x80 | ((code >> 6) & 63), 0x80 | (code & 63));
                    i++;
                } else { out.push(0xEF, 0xBF, 0xBD); }
            } else { out.push(0xE0 | (code >> 12), 0x80 | ((code >> 6) & 63), 0x80 | (code & 63)); }
        }
        return new Uint8Array(out);
    };
    W.TextEncoder = TextEncoderCtor;

    function TextDecoderCtor() { }
    TextDecoderCtor.prototype.decode = function (bytes) {
        var b = bytes || [];
        var out = "";
        var i = 0;
        while (i < b.length) {
            var c = b[i];
            if (c < 0x80) { out += String.fromCharCode(c); i += 1; }
            else if (c < 0xE0) {
                out += String.fromCharCode(((c & 31) << 6) | (b[i + 1] & 63));
                i += 2;
            } else if (c < 0xF0) {
                out += String.fromCharCode(((c & 15) << 12) | ((b[i + 1] & 63) << 6) | (b[i + 2] & 63));
                i += 3;
            } else {
                var cp = ((c & 7) << 18) | ((b[i + 1] & 63) << 12) | ((b[i + 2] & 63) << 6) | (b[i + 3] & 63);
                cp -= 0x10000;
                out += String.fromCharCode(0xD800 + (cp >> 10), 0xDC00 + (cp & 0x3FF));
                i += 4;
            }
        }
        return out;
    };
    W.TextDecoder = TextDecoderCtor;

    function BlobCtor(parts, opts) {
        var size = 0;
        parts = parts || [];
        for (var i = 0; i < parts.length; i++) {
            size += (parts[i] && parts[i].length) || 0;
        }
        this.size = size;
        this.type = (opts && opts.type) || "";
    }
    BlobCtor.prototype.slice = function () { return new BlobCtor([], { type: this.type }); };
    BlobCtor.prototype.text = function () { return Promise.resolve(""); };
    BlobCtor.prototype.arrayBuffer = function () { return Promise.resolve(new ArrayBuffer(0)); };
    W.Blob = BlobCtor;

    function FormDataCtor() {
        this._pairs = [];
    }
    pairMethods(FormDataCtor);
    FormDataCtor.prototype.append = function (k, v) { this._pairs.push([String(k), v === undefined ? "" : String(v)]); };
    FormDataCtor.prototype.set = function (k, v) {
        for (var i = 0; i < this._pairs.length; i++) {
            if (this._pairs[i][0] === String(k)) { this._pairs[i][1] = v === undefined ? "" : String(v); return; }
        }
        this.append(k, v);
    };
        W.FormData = FormDataCtor;

    function AbortSignalLike() {
        this.aborted = false;
        this.onabort = null;
        this.reason = undefined;
        var self = this;
        this._listeners = [];
        this.addEventListener = function (t, fn) { if (t === "abort" && typeof fn === "function") { self._listeners.push(fn); } };
        this.removeEventListener = function (t, fn) {
            var l = self._listeners;
            for (var i = 0; i < l.length; i++) { if (l[i] === fn) { l.splice(i, 1); return; } }
        };
        this.dispatchEvent = function () { return true; };
        this._fire = function (reason) {
            if (self.aborted) { return; }
            self.aborted = true;
            self.reason = reason;
            if (typeof self.onabort === "function") { try { self.onabort(self); } catch (e) { } }
            var l = self._listeners.slice();
            self._listeners.length = 0;
            for (var i = 0; i < l.length; i++) { try { l[i](self); } catch (e) { } }
        };
        this._follow = function (other) {
            if (other.aborted) { self._fire(other.reason); return; }
            other.addEventListener("abort", function () { self._fire(other.reason); });
        };
    }
    function AbortControllerCtor() {
        this.signal = new AbortSignalLike();
        this.abort = function (reason) { this.signal._fire(reason); };
    }
    W.AbortController = AbortControllerCtor;
    var AbortSignalCtor = function () { throw new TypeError("Illegal constructor"); };
    AbortSignalCtor.prototype = AbortSignalLike.prototype;
    AbortSignalCtor.abort = function (reason) { var s = new AbortSignalLike(); s._fire(reason); return s; };
    AbortSignalCtor.timeout = function (ms) { var s = new AbortSignalLike(); W.setTimeout(function () { s._fire(new DOMExceptionLike("signal timed out")); }, ms); return s; };
    AbortSignalCtor.any = function (signals) {
        var out = new AbortSignalLike();
        for (var i = 0; i < signals.length; i++) { out._follow(signals[i]); }
        return out;
    };
    W.AbortSignal = AbortSignalCtor;

    function MessagePortLike() {
        this.onmessage = null;
        this.onmessageerror = null;
        var self = this;
        this._q = [];
        this._other = null;
        this.postMessage = function (m) {
            var o = self._other;
            if (!o) { return; }
            var ev = makeEvent("message", { target: o, currentTarget: o, data: m, origin: W.location ? W.location.origin : "null", ports: [] });
            if (typeof o.onmessage === "function") { try { o.onmessage(ev); } catch (e) { } }
        };
        this.start = function () { };
        this.close = function () { self._other = null; };
        this.addEventListener = function (t, fn) { if (t === "message" && typeof fn === "function") { self.onmessage = fn; } };
        this.removeEventListener = function () { };
    }
    function MessageChannelCtor() {
        var a = new MessagePortLike();
        var b = new MessagePortLike();
        a._other = b;
        b._other = a;
        this.port1 = a;
        this.port2 = b;
    }
    W.MessageChannel = MessageChannelCtor;
    var MessagePortCtor = function () { throw new TypeError("Illegal constructor"); };
    MessagePortCtor.prototype = MessagePortLike.prototype;
    W.MessagePort = MessagePortCtor;
    var BroadcastChannelCtor = function (name) { this.name = String(name); this.onmessage = null; };
    BroadcastChannelCtor.prototype.postMessage = function () { };
    BroadcastChannelCtor.prototype.close = function () { };
    BroadcastChannelCtor.prototype.addEventListener = function () { };
    BroadcastChannelCtor.prototype.removeEventListener = function () { };
    W.BroadcastChannel = BroadcastChannelCtor;

    function WebSocketCtor(url, protocols) {
        url = String(url);
        var m = /^(wss?):\/\/([^\/?#]+)([^?#]*)(\?[^#]*)?(#.*)?/.exec(url);
        if (!m) { throw new SyntaxError("Failed to construct 'WebSocket': The URL '" + url + "' is invalid."); }
        var self = this;
        this.url = url;
        this.protocol = "";
        this.extensions = "";
        this.readyState = 0;
        this.bufferedAmount = 0;
        this.binaryType = "blob";
        this.onopen = null;
        this.onclose = null;
        this.onerror = null;
        this.onmessage = null;
        this.send = function () { };
        this.close = function () { self.readyState = 3; };
        W.setTimeout(function () {
            self.readyState = 3;
            var cev = makeEvent("close", { target: self, currentTarget: self, wasClean: true, code: 1006, reason: "" });
            var eev = makeEvent("error", { target: self, currentTarget: self, bubbles: false });
            if (typeof self.onerror === "function") { try { self.onerror(eev); } catch (e) { } }
            if (typeof self.onclose === "function") { try { self.onclose(cev); } catch (e) { } }
        }, 0);
    }
    WebSocketCtor.CONNECTING = 0;
    WebSocketCtor.OPEN = 1;
    WebSocketCtor.CLOSING = 2;
    WebSocketCtor.CLOSED = 3;
    WebSocketCtor.prototype.send = function () { };
    WebSocketCtor.prototype.close = function () { };
    W.WebSocket = WebSocketCtor;

    function EventSourceCtor(url) {
        var self = this;
        this.url = String(url);
        this.readyState = 0;
        this.withCredentials = false;
        this.onopen = null;
        this.onmessage = null;
        this.onerror = null;
        this.close = function () { self.readyState = 2; };
        this.addEventListener = function () { };
        this.removeEventListener = function () { };
        W.setTimeout(function () {
            self.readyState = 2;
            var ev = makeEvent("error", { target: self, currentTarget: self, bubbles: false });
            if (typeof self.onerror === "function") { try { self.onerror(ev); } catch (e) { } }
        }, 0);
    }
    EventSourceCtor.prototype.close = function () { };
    EventSourceCtor.CONNECTING = 0;
    EventSourceCtor.OPEN = 1;
    EventSourceCtor.CLOSED = 2;
    W.EventSource = EventSourceCtor;

    function DOMExceptionLike(message, name) {
        this.message = message === undefined ? "" : String(message);
        this.name = name === undefined ? "Error" : String(name);
    }
    DOMExceptionLike.prototype = Object.create(Error.prototype);
    DOMExceptionLike.prototype.constructor = DOMExceptionLike;
    DOMExceptionLike.prototype.toString = function () { return this.name + ": " + this.message; };
    var DEXC = W.DOMException;
    if (typeof DEXC === "function") {
        DOMExceptionLike.prototype = DEXC.prototype;
    }

    function FileCtor(parts, name, opts) {
        BlobCtor.call(this, parts, opts);
        this.name = String(name);
        this.lastModified = (opts && opts.lastModified) || 0;
    }
    FileCtor.prototype = Object.create(BlobCtor.prototype);
    FileCtor.prototype.constructor = FileCtor;
    W.File = FileCtor;

    function FileReaderCtor() {
        var self = this;
        this.readyState = 0;
        this.result = null;
        this.error = null;
        this.onload = null;
        this.onerror = null;
        this.onloadend = null;
        this.onabort = null;
        this.onprogress = null;
        this.abort = function () { self.readyState = 2; };
        function done(res) {
            self.readyState = 2;
            self.result = res;
            if (typeof self.onload === "function") { try { self.onload(makeEvent("load", { target: self, currentTarget: self })); } catch (e) { } }
            if (typeof self.onloadend === "function") { try { self.onloadend(makeEvent("loadend", { target: self, currentTarget: self })); } catch (e) { } }
        }
        this.readAsText = function () { W.setTimeout(function () { done(""); }, 0); };
        this.readAsDataURL = function () { W.setTimeout(function () { done("data:;base64,"); }, 0); };
        this.readAsArrayBuffer = function () { W.setTimeout(function () { done(new ArrayBuffer(0)); }, 0); };
        this.readAsBinaryString = function () { W.setTimeout(function () { done(""); }, 0); };
    }
    W.FileReader = FileReaderCtor;

    function ImageCtor(w, h) {
        var self = this;
        this.naturalWidth = 0;
        this.naturalHeight = 0;
        this.complete = false;
        this.onload = null;
        this.onerror = null;
        if (w !== undefined) { this.width = Number(w) || 0; }
        if (h !== undefined) { this.height = Number(h) || 0; }
        var src = "";
        Object.defineProperty(this, "src", {
            get: function () { return src; },
            set: function (v) {
                src = String(v);
                if (/^data:image\//i.test(src)) {
                    W.setTimeout(function () {
                        self.complete = true;
                        if (typeof self.onload === "function") { try { self.onload(makeEvent("load", { target: self, currentTarget: self })); } catch (e) { } }
                    }, 0);
                }
            },
            configurable: true, enumerable: true
        });
    }
    W.Image = ImageCtor;

    function OptionCtor(text, value, defaultSelected, selected) {
        this.text = text === undefined ? "" : String(text);
        this.value = value === undefined ? this.text : String(value);
        this.defaultSelected = !!defaultSelected;
        this.selected = !!selected;
    }
    W.Option = OptionCtor;

    function Path2DCtor() { }
    ["moveTo", "lineTo", "closePath", "arc", "arcTo", "rect", "ellipse", "bezierCurveTo", "quadraticCurveTo"].forEach(function (m) {
        Path2DCtor.prototype[m] = function () { };
    });
    W.Path2D = Path2DCtor;

    function pairIter() {
        var out = [];
        for (var i = 0; i < this._h.length; i++) { out.push([this._h[i], this._v[i]]); }
        return out[Symbol.iterator]();
    }
    function HeadersCtor(init) {
        this._h = [];
        this._v = [];
        var self = this;
        function add(k, v) { self.set(k, v); }
        if (init) {
            if (typeof init.forEach === "function") { init.forEach(add); }
            else if (Array.isArray(init)) { for (var i = 0; i < init.length; i++) { add(init[i][0], init[i][1]); } }
            else { for (var k in init) { if (Object.prototype.hasOwnProperty.call(init, k)) { add(k, init[k]); } } }
        }
    }
    HeadersCtor.prototype.append = function (k, v) { this._h.push(String(k).toLowerCase()); this._v.push(String(v)); };
    HeadersCtor.prototype.set = function (k, v) {
        k = String(k).toLowerCase();
        for (var i = 0; i < this._h.length; i++) {
            if (this._h[i] === k) { this._v[i] = String(v); return; }
        }
        this.append(k, v);
    };
    HeadersCtor.prototype.get = function (k) {
        k = String(k).toLowerCase();
        for (var i = 0; i < this._h.length; i++) { if (this._h[i] === k) { return this._v[i]; } }
        return null;
    };
    HeadersCtor.prototype.has = function (k) {
        k = String(k).toLowerCase();
        for (var i = 0; i < this._h.length; i++) { if (this._h[i] === k) { return true; } }
        return false;
    };
    HeadersCtor.prototype.delete = function (k) {
        k = String(k).toLowerCase();
        for (var i = 0; i < this._h.length; i++) {
            if (this._h[i] === k) { this._h.splice(i, 1); this._v.splice(i, 1); return; }
        }
    };
    HeadersCtor.prototype.forEach = function (fn) { for (var i = 0; i < this._h.length; i++) { fn(this._v[i], this._h[i]); } };
    HeadersCtor.prototype.entries = pairIter;
    HeadersCtor.prototype.keys = function () { var out = this._h.slice(); return out[Symbol.iterator](); };
    HeadersCtor.prototype.values = function () { var out = this._v.slice(); return out[Symbol.iterator](); };
    W.Headers = HeadersCtor;

    function RequestCtor(input, init) {
        init = init || {};
        if (input && typeof input === "object") {
            this.url = String(input.url || "");
            this.method = String(init.method || input.method || "GET").toUpperCase();
            this.headers = new HeadersCtor(init.headers || input.headers);
            this.body = init.body || null;
        } else {
            this.url = String(input || "");
            this.method = String(init.method || "GET").toUpperCase();
            this.headers = new HeadersCtor(init.headers);
            this.body = init.body || null;
        }
        this.credentials = init.credentials || "same-origin";
        this.mode = init.mode || "cors";
        this.redirect = init.redirect || "follow";
        this.cache = init.cache || "default";
        this.destination = "";
        this.referrer = "about:client";
        this.referrerPolicy = "";
        this.signal = (init.signal instanceof AbortSignalLike) ? init.signal : null;
        this.integrity = "";
        this.isReloadNavigation = false;
        this.keepalive = false;
    }
    RequestCtor.prototype.clone = function () { return new RequestCtor(this.url, { method: this.method, headers: this.headers, body: this.body }); };
    W.Request = RequestCtor;

    function ResponseCtor(body, init) {
        init = init || {};
        this.ok = (init.status || 200) >= 200 && (init.status || 200) < 300;
        this.status = init.status || 200;
        this.statusText = init.statusText || "";
        this.headers = new HeadersCtor(init.headers);
        this.body = body || null;
        this.type = "basic";
        this.redirected = false;
        this.url = "";
        this.bodyUsed = false;
        var self = this;
        function readText() {
            if (self.bodyUsed) { return Promise.reject(new TypeError("body used already")); }
            self.bodyUsed = true;
            var t = "";
            if (typeof self.body === "string") { t = self.body; }
            else if (self.body && self.body.text) { return self.body.text(); }
            return Promise.resolve(t);
        }
        this.text = function () { return readText(); };
        this.json = function () { return readText().then(function (t) { return JSON.parse(t); }); };
        this.arrayBuffer = function () { return this.text().then(function (t) { return new ArrayBuffer(0); }); };
        this.blob = function () { return this.text().then(function () { return new BlobCtor([], {}); }); };
        this.clone = function () { return new ResponseCtor(self.body, { status: self.status, statusText: self.statusText, headers: self.headers }); };
    }
    ResponseCtor.error = function () { return new ResponseCtor(null, { status: 0 }); };
    ResponseCtor.redirect = function (url, status) { return new ResponseCtor(null, { status: status || 302, headers: [["location", url]] }); };
    W.Response = ResponseCtor;

    function DOMRectCtor(x, y, w, h) {
        this.x = +x || 0; this.y = +y || 0;
        this.width = +w || 0; this.height = +h || 0;
        this.top = this.y; this.bottom = this.y + this.height;
        this.left = this.x; this.right = this.x + this.width;
    }
    W.DOMRect = DOMRectCtor;
    var DOMRectReadOnlyCtor = function (x, y, w, h) { DOMRectCtor.call(this, x, y, w, h); };
    DOMRectReadOnlyCtor.prototype = Object.create(DOMRectCtor.prototype);
    W.DOMRectReadOnly = DOMRectReadOnlyCtor;
    function DOMPointCtor(x, y, z, w) {
        this.x = +x || 0; this.y = +y || 0; this.z = +z || 0; this.w = +w || 0;
    }
    W.DOMPoint = DOMPointCtor;
    var DOMPointReadOnlyCtor = function (x, y, z, w) { DOMPointCtor.call(this, x, y, z, w); };
    DOMPointReadOnlyCtor.prototype = Object.create(DOMPointCtor.prototype);
    W.DOMPointReadOnly = DOMPointReadOnlyCtor;
    function DOMMatrixCtor(init) {
        this.a = 1; this.b = 0; this.c = 0; this.d = 1; this.e = 0; this.f = 0;
        this.m11 = 1; this.m12 = 0; this.m13 = 0; this.m14 = 0;
        this.m21 = 0; this.m22 = 1; this.m23 = 0; this.m24 = 0;
        this.m31 = 0; this.m32 = 0; this.m33 = 1; this.m34 = 0;
        this.m41 = 0; this.m42 = 0; this.m43 = 0; this.m44 = 1;
        this.is2D = true; this.isIdentity = true;
        if (init) { if (init.a !== undefined) { this.a = init.a; this.d = init.d || 1; this.e = init.e || 0; this.f = init.f || 0; this.isIdentity = false; } }
    }
    DOMMatrixCtor.prototype.multiply = function (o) { return new DOMMatrixCtor(o); };
    DOMMatrixCtor.prototype.scale = function () { return this; };
    DOMMatrixCtor.prototype.translate = function () { return this; };
    DOMMatrixCtor.prototype.invert = function () { return this; };
    W.DOMMatrix = DOMMatrixCtor;
    var DOMMatrixReadOnlyCtor = function (init) { DOMMatrixCtor.call(this, init); };
    DOMMatrixReadOnlyCtor.prototype = Object.create(DOMMatrixCtor.prototype);
    W.DOMMatrixReadOnly = DOMMatrixReadOnlyCtor;
    function DOMQuadCtor(p1, p2, p3, p4) {
        this.p1 = p1 || new DOMPointCtor(0, 0);
        this.p2 = p2 || new DOMPointCtor(0, 0);
        this.p3 = p3 || new DOMPointCtor(0, 0);
        this.p4 = p4 || new DOMPointCtor(0, 0);
    }
    W.DOMQuad = DOMQuadCtor;

    W.NodeFilter = {
        SHOW_ALL: 0xFFFFFFFF, SHOW_ELEMENT: 1, SHOW_ATTRIBUTE: 2, SHOW_TEXT: 4,
        SHOW_CDATA_SECTION: 8, SHOW_ENTITY_REFERENCE: 16, SHOW_ENTITY: 32,
        SHOW_PROCESSING_INSTRUCTION: 64, SHOW_COMMENT: 128, SHOW_DOCUMENT: 256,
        SHOW_DOCUMENT_TYPE: 512, SHOW_DOCUMENT_FRAGMENT: 1024, SHOW_NOTATION: 2048,
        FILTER_ACCEPT: 1, FILTER_REJECT: 2, FILTER_SKIP: 3
    };

    function PerformanceObserverCtor(cb) {
        this._cb = cb;
        var self = this;
        this.observe = function () { };
        this.unobserve = function () { };
        this.disconnect = function () { };
        this.takeRecords = function () { return []; };
    }
    W.PerformanceObserver = PerformanceObserverCtor;

    if (W.performance) {
        W.performance.mark = function (name) {
            return { entryType: "mark", name: String(name), startTime: W.performance.now(), duration: 0 };
        };
        W.performance.measure = function (name) {
            return { entryType: "measure", name: String(name), startTime: W.performance.now(), duration: 0 };
        };
        W.performance.getEntries = function () { return []; };
        W.performance.getEntriesByName = function () { return []; };
        W.performance.getEntriesByType = function () { return []; };
    }

    function SpeechSynthesisUtteranceCtor(text) {
        this.text = text === undefined ? "" : String(text);
        this.lang = "";
        this.voice = null;
        this.volume = 1;
        this.rate = 1;
        this.pitch = 1;
        this.onstart = null;
        this.onend = null;
        this.onerror = null;
    }
    W.SpeechSynthesisUtterance = SpeechSynthesisUtteranceCtor;
    var speechSynthesisObj = {
        speaking: false, pending: false, paused: false,
        getVoices: function () { return []; },
        speak: function () { },
        cancel: function () { },
        pause: function () { },
        resume: function () { },
        addEventListener: function () { },
        removeEventListener: function () { }
    };
    if (typeof W.SpeechSynthesis === "function") {
        Object.setPrototypeOf(speechSynthesisObj, W.SpeechSynthesis.prototype);
    }
    W.speechSynthesis = speechSynthesisObj;

    try { Object.setPrototypeOf(W.history, typeof W.History === "function" ? W.History.prototype : Object.prototype); } catch (e) { }
    try { Object.setPrototypeOf(W.localStorage, typeof W.Storage === "function" ? W.Storage.prototype : Object.prototype); } catch (e) { }
    try { Object.setPrototypeOf(W.sessionStorage, typeof W.Storage === "function" ? W.Storage.prototype : Object.prototype); } catch (e) { }
    try { Object.setPrototypeOf(orientation, typeof W.ScreenOrientation === "function" ? W.ScreenOrientation.prototype : Object.prototype); } catch (e) { }

    try {
        var DROP = { InternalError: 1, SharedArrayBuffer: 1 };
        var SKIP = { undefined: 1, NaN: 1, Infinity: 1 };
        var gn = Object.getOwnPropertyNames(globalThis);
        for (var gi = 0; gi < gn.length; gi++) {
            var gname = gn[gi];
            if (DROP[gname]) {
                try { delete globalThis[gname]; } catch (e) { }
                continue;
            }
            if (SKIP[gname]) { continue; }
            try {
                var gd = Object.getOwnPropertyDescriptor(globalThis, gname);
                if (gd && !gd.enumerable) {
                    if (gd.get || gd.set) {
                        Object.defineProperty(globalThis, gname, { enumerable: true, configurable: true, get: gd.get, set: gd.set });
                    } else {
                        Object.defineProperty(globalThis, gname, { enumerable: true, configurable: true, writable: !!gd.writable, value: gd.value });
                    }
                }
            } catch (e) { }
        }
    } catch (e) { }
    W.__silo_feed = function (buf) {
        var u8 = new Uint8Array(buf);
        for (var i = 0; i + 8 <= u8.length; i += 8) {
            var x = u8[i] | (u8[i + 1] << 8);
            var y = u8[i + 2] | (u8[i + 3] << 8);
            var dt = u8[i + 4] | (u8[i + 5] << 8);
            feed(x, y, dt, u8[i + 6], u8[i + 7]);
        }
    };
    W.__silo_reset = reset;
    W.__silo_stub_fetch = W.fetch;
    var IDLE_SEQ = 1;
    var idleDequeued = {};
    W.requestIdleCallback = function (cb, opts) {
        var id = IDLE_SEQ++;
        var timeout = (opts && opts.timeout) || 50;
        var handle = function () {
            if (idleDequeued[id]) { return; }
            idleDequeued[id] = true;
            try {
                cb({
                    didTimeout: false,
                    timeRemaining: function () { return 49; }
                });
            } catch (e) { }
        };
        W.setTimeout(handle, Math.min(Math.max(timeout, 1), 50));
        return id;
    };
    W.cancelIdleCallback = function (id) { idleDequeued[id] = true; };
});
