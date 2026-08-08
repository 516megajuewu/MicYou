;; MicYou WASM example: wasm-http (network tester)
;; Panel button -> http_request(GET https://api.github.com/zen)
;; -> http:response -> notify + persist lastCheck via set_config
(module
  (import "micyou" "log" (func $log (param i32 i32)))
  (import "micyou" "http_request" (func $http_request (param i32 i32 i32 i32) (result i64)))
  (import "micyou" "notify" (func $notify (param i32 i32)))
  (import "micyou" "set_config" (func $set_config (param i32 i32) (result i32)))
  (import "micyou" "set_panel_icon" (func $set_panel_icon (param i32 i32)))
  (memory (export "memory") 4)
  ;; bump allocator
  (global $heap (mut i32) (i32.const 0x3000))
  (func (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $n)))
    (i32.store (local.get $p) (local.get $n))
    (i32.add (local.get $p) (i32.const 4)))
  (func (export "dealloc") (param $p i32) (param $n i32))
  (func (export "api_version") (result i32) (i32.const 1))

  ;; statics
  ;; 0x100 'ping'           0x108 '"ok":'    0x110 'true'
  ;; 0x118 'GET'            0x11C '{}'       0x120 url
  ;; 0x160 'lastCheck'      0x16C '"ok"'     0x171 '"fail"'
  ;; 0x180 title 网络测试    0x190 请求成功    0x1A0 请求失败
  ;; 0x1B0 'control'        0x1B8 🌐 emoji
  (data (i32.const 0x100) "ping\00")
  (data (i32.const 0x108) "\22ok\22:")
  (data (i32.const 0x110) "true")
  (data (i32.const 0x118) "GET\00")
  (data (i32.const 0x11C) "{}\00")
  (data (i32.const 0x120) "https://api.github.com/zen\00")
  (data (i32.const 0x160) "lastCheck\00")
  (data (i32.const 0x16C) "\22ok\22")
  (data (i32.const 0x171) "\22fail\22")
  (data (i32.const 0x180) "\E7\BD\91\E7\BB\9C\E6\B5\8B\E8\AF\95\00")   ;; 网络测试
  (data (i32.const 0x190) "\E8\AF\B7\E6\B1\82\E6\88\90\E5\8A\9F\00")   ;; 请求成功
  (data (i32.const 0x1A0) "\E8\AF\B7\E6\B1\82\E5\A4\B1\E8\B4\A5\00")   ;; 请求失败
  (data (i32.const 0x1B0) "control\00")
  (data (i32.const 0x1B8) "\F0\9F\8C\90")
  (data (i32.const 0x1C0) "http tester ready\00")                              ;; 🌐

  ;; REQID global at 0x1F0
  (global $reqid (mut i64) (i64.const 0))

  (func $strlen (param $p i32) (result i32)
    (local $i i32)
    (block $out
      (loop $lp
        (br_if $out (i32.eqz (i32.load8_u (i32.add (local.get $p) (local.get $i)))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)))
    (local.get $i))

  ;; contains(hay, needle) -> i32  (mismatch sets flag + breaks, full match leaves 0)
  (func $contains (param $hay i32) (param $hlen i32) (param $needle i32) (result i32)
    (local $nl i32) (local $i i32) (local $j i32) (local $found i32)
    (local.set $nl (call $strlen (local.get $needle)))
    (if (i32.eqz (local.get $nl)) (then (return (i32.const 1))))
    (local.set $i (i32.const 0))
    (block $outer
      (loop $outer_lp
        (br_if $outer (i32.gt_u (local.get $i) (i32.sub (local.get $hlen) (local.get $nl))))
        (local.set $found (i32.const 0))
        (local.set $j (i32.const 0))
        (block $inner
          (loop $inner_lp
            (br_if $inner (i32.ge_u (local.get $j) (local.get $nl)))
            (if (i32.ne
                  (i32.load8_u (i32.add (local.get $hay) (i32.add (local.get $i) (local.get $j))))
                  (i32.load8_u (i32.add (local.get $needle) (local.get $j))))
              (then (local.set $found (i32.const 1)) (br $inner)))
            (local.set $j (i32.add (local.get $j) (i32.const 1)))
            (br $inner_lp)))
        (if (i32.eqz (local.get $found)) (then (return (i32.const 1))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $outer_lp)))
    (i32.const 0))

  (func (export "init") (result i32)
    (call $set_panel_icon (i32.const 0x1B0) (i32.const 0x1B8))
    (call $log (i32.const 2) (i32.const 0x1C0))
    (i32.const 0))

  (func (export "handle_message") (param $payload i32) (param $len i32) (result i32)
    (local $ok i32)
    (if (call $contains (local.get $payload) (local.get $len) (i32.const 0x100))
      (then
        (call $log (i32.const 2) (i32.const 0x1C0))
        (drop (call $http_request (i32.const 0x118) (i32.const 0x120) (i32.const 0x11C) (i32.const 0)))
        (return (i32.const 0))))
    ;; http:response payload {request, ok, status, body, error}
    (if (call $contains (local.get $payload) (local.get $len) (i32.const 0x108))
      (then
        (local.set $ok (call $contains (local.get $payload) (local.get $len) (i32.const 0x110)))
        (if (local.get $ok)
          (then
            (drop (call $set_config (i32.const 0x160) (i32.const 0x16C)))
            (call $notify (i32.const 0x180) (i32.const 0x190)))
          (else
            (drop (call $set_config (i32.const 0x160) (i32.const 0x171)))
            (call $notify (i32.const 0x180) (i32.const 0x1A0))))))
    (i32.const 0))

  (func (export "deinit"))

)
