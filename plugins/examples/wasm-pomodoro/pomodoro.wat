;; MicYou WASM pomodoro timer plugin
;; Imports: log / notify / set_interval / clear_interval
;; Exports: memory / alloc / dealloc / api_version / init / handle_message / deinit
;;
;; State machine (all in linear memory):
;;   0x100 COUNT i32   seconds remaining
;;   0x104 MODE i32    0=idle 1=work 2=break
;;   0x108 TIMER i64   interval handle
;;
;; Handle_message payloads (self-describing, no topic):
;;   {"action":"start"}            -> start a 25-minute work session
;;   {"action":"stop"}             -> stop the current session
;;   {"interval":<id>,"payload":""}-> one-second tick from the host timer
(module
  (import "micyou" "log" (func $log (param i32 i32)))
  (import "micyou" "notify" (func $notify (param i32 i32)))
  (import "micyou" "set_interval" (func $set_interval (param i64 i32) (result i64)))
  (import "micyou" "clear_interval" (func $clear_interval (param i64)))
  (import "micyou" "get_config" (func $get_config (param i32) (result i32)))
  (import "micyou" "set_config" (func $set_config (param i32 i32) (result i32)))
  (import "micyou" "set_panel_icon" (func $set_panel_icon (param i32 i32)))
  (memory (export "memory") 4)

  ;; static data
  ;; 0x100 COUNT (i32)
  ;; 0x104 MODE (i32)
  ;; 0x108 TIMER (i64)
  ;; 0x110 "番茄钟" (9 bytes + NUL)
  (data (i32.const 0x110) "\E7\95\AA\E8\8C\84\E9\92\9F\00")
  ;; 0x120 "时间到！休息一下吧" (24 bytes + NUL)
  (data (i32.const 0x120) "\E6\97\B6\E9\97\B4\E5\88\B0\EF\BC\81\E4\BC\91\E6\81\AF\E4\B8\80\E4\B8\8B\E5\90\A7\00")
  ;; 0x140 "休息结束，开始工作吧" (30 bytes + NUL)
  (data (i32.const 0x140) "\E4\BC\91\E6\81\AF\E7\BB\93\E6\9D\9F\EF\BC\8C\E5\BC\80\E5\A7\8B\E5\B7\A5\E4\BD\9C\E5\90\A7\00")
  ;; 0x160 "action"
  (data (i32.const 0x160) "action\00")
  ;; 0x168 "stop"
  (data (i32.const 0x168) "stop\00")
  ;; 0x170 "pomodoro initialized"
  (data (i32.const 0x170) "pomodoro initialized\00")
  ;; 0x190 "pomodoro stopped"
  (data (i32.const 0x190) "pomodoro stopped\00")
  ;; 0x1A4 "pomodoro started (25 min)"
  (data (i32.const 0x1A4) "pomodoro started (25 min)\00")
  ;; 0x1C0 "" (empty payload string for set_interval)
  (data (i32.const 0x1C0) "\00")
  ;; 0x1D0 "开始工作"（start 即时反馈通知）
  (data (i32.const 0x1D0) "\E5\BC\80\E5\A7\8B\E5\B7\A5\E4\BD\9C\00")
  ;; 0x1C1 "pomodoro tick"
  (data (i32.const 0x1C1) "pomodoro tick\00")
  ;; 0x1E0 "workMin" / 0x1E8 "breakMin" / 0x1F2 "mode" / 0x1F8 "work" / 0x1FE "break" / 0x204 "idle"
  (data (i32.const 0x1E0) "workMin\00")
  (data (i32.const 0x1E8) "breakMin\00")
  (data (i32.const 0x1F2) "mode\00")
  (data (i32.const 0x1F8) "\22work\22\00")
  (data (i32.const 0x200) "\22break\22\00")
  (data (i32.const 0x208) "\22idle\22\00")
  ;; 0x210 "control" / 0x218 "🍅" (F0 9F 8D 85)
  (data (i32.const 0x210) "control\00")
  (data (i32.const 0x218) "\F0\9F\8D\85\00")

  ;; bump allocator (heap starts after all statics)
  (global $heap (mut i32) (i32.const 0x2200))
  (func (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $n)))
    (i32.store (local.get $p) (local.get $n))
    (i32.add (local.get $p) (i32.const 4)))
  (func (export "dealloc") (param $p i32) (param $n i32))

  (func (export "api_version") (result i32)
    (i32.const 1))

  (func (export "init") (result i32)
    (i32.store (i32.const 0x100) (i32.const 0))
    (i32.store (i32.const 0x104) (i32.const 0))
    (i64.store (i32.const 0x108) (i64.const 0))
    (call $set_panel_icon (i32.const 0x210) (i32.const 0x218))
    (call $log (i32.const 2) (i32.const 0x170))
    (i32.const 0))

  ;; contains(h, hl, n, nl) -> i32 : 1 if n is a substring of h
  (func $contains (param $h i32) (param $hl i32) (param $n i32) (param $nl i32) (result i32)
    (local $i i32) (local $j i32) (local $found i32)
    (block $out (result i32)
      (local.set $found (i32.const 0))
      (local.set $i (i32.const 0))
      (block $outer
        (loop $l
          (br_if $outer (i32.ge_u (local.get $i) (local.get $hl)))
          (br_if $outer (local.get $found))
          (local.set $j (i32.const 0))
          (block $inner
            (loop $m
              (if (i32.ge_u (local.get $j) (local.get $nl))
                (then
                  (local.set $found (i32.const 1))
                  (br $inner)))
              (if (i32.ne
                    (i32.load8_u (i32.add (local.get $h) (i32.add (local.get $i) (local.get $j))))
                    (i32.load8_u (i32.add (local.get $n) (local.get $j))))
                (then (br $inner)))
              (local.set $j (i32.add (local.get $j) (i32.const 1)))
              (br $m)))
          (local.set $i (i32.add (local.get $i) (i32.const 1)))
          (br $l)))
      (local.get $found)
      (br $out)))

  ;; is_digit(c) -> i32 (0/1)
  (func $is_digit (param $c i32) (result i32)
    (i32.and
      (i32.ge_u (local.get $c) (i32.const 48))
      (i32.le_u (local.get $c) (i32.const 57))))

  ;; read_minutes(key_ptr, fallback_seconds) -> seconds
  ;; reads a config value via get_config, parses the first integer (minutes)
  ;; and converts to seconds; falls back when absent
  (func $read_minutes (param $key i32) (param $fallback i32) (result i32)
    (local $p i32) (local $n i32) (local $v i32)
    (local.set $p (call $get_config (local.get $key)))
    (if (i32.eqz (local.get $p))
      (then (return (local.get $fallback))))
    (local.set $n (i32.load8_u (local.get $p)))
    (block $scan
      (loop $s
        (if (call $is_digit (local.get $n)) (then (br $scan)))
        (local.set $p (i32.add (local.get $p) (i32.const 1)))
        (local.set $n (i32.load8_u (local.get $p)))
        (br $s)))
    (local.set $v (i32.const 0))
    (block $collect
      (loop $d
        (if (call $is_digit (local.get $n))
          (then
            (local.set $v (i32.add (i32.mul (local.get $v) (i32.const 10))
              (i32.sub (local.get $n) (i32.const 48))))
            (local.set $p (i32.add (local.get $p) (i32.const 1)))
            (local.set $n (i32.load8_u (local.get $p)))
            (br $d))
          (else (br $collect)))))
    (i32.mul (local.get $v) (i32.const 60)))

  (func (export "handle_message") (param $ptr i32) (param $len i32) (result i32)
    ;; UI command branch: payload contains "action"
    (if (call $contains (local.get $ptr) (local.get $len) (i32.const 0x160) (i32.const 6))
      (then
        (if (call $contains (local.get $ptr) (local.get $len) (i32.const 0x168) (i32.const 4))
          (then
            ;; stop
            (call $clear_interval (i64.load (i32.const 0x108)))
            (i32.store (i32.const 0x104) (i32.const 0))
            (i32.store (i32.const 0x100) (i32.const 0))
            (drop (call $set_config (i32.const 0x1F2) (i32.const 0x208)))
            (call $log (i32.const 2) (i32.const 0x190)))
          (else
            ;; start: read workMin from config (default 25 min), arm the timer
            (i32.store (i32.const 0x100)
              (call $read_minutes (i32.const 0x1E0) (i32.const 1500)))
            (i32.store (i32.const 0x104) (i32.const 1))
            (drop (call $set_config (i32.const 0x1F2) (i32.const 0x1F8)))
            (i64.store (i32.const 0x108)
              (call $set_interval (i64.const 1000) (i32.const 0x1C0)))
            (call $notify (i32.const 0x110) (i32.const 0x1D0))
            (call $log (i32.const 2) (i32.const 0x1A4)))))
      (else
        ;; interval tick
        (call $log (i32.const 4) (i32.const 0x1C1))
        (i32.store (i32.const 0x100)
          (i32.sub (i32.load (i32.const 0x100)) (i32.const 1)))
        (if (i32.le_s (i32.load (i32.const 0x100)) (i32.const 0))
          (then
            (if (i32.eq (i32.load (i32.const 0x104)) (i32.const 1))
              (then
                ;; work finished -> notify, switch to a break (default 5 min)
                (call $notify (i32.const 0x110) (i32.const 0x120))
                (i32.store (i32.const 0x104) (i32.const 2))
                (i32.store (i32.const 0x100)
                  (call $read_minutes (i32.const 0x1E8) (i32.const 300)))
                (drop (call $set_config (i32.const 0x1F2) (i32.const 0x200)))
                (i64.store (i32.const 0x108)
                  (call $set_interval (i64.const 1000) (i32.const 0x1C0))))
              (else
                (if (i32.eq (i32.load (i32.const 0x104)) (i32.const 2))
                  (then
                    ;; break finished -> notify, back to idle
                    (call $notify (i32.const 0x110) (i32.const 0x140))
                    (i32.store (i32.const 0x104) (i32.const 0))
                    (drop (call $set_config (i32.const 0x1F2) (i32.const 0x208)))
                    (call $clear_interval (i64.load (i32.const 0x108))))
                  (else
                    ;; idle tick without a session: stop any stray timer
                    (call $clear_interval (i64.load (i32.const 0x108)))))))))))
    (i32.const 0))

  (func (export "deinit"))
)
