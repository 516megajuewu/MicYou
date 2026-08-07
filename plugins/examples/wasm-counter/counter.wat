;; MicYou WASM plugin example — 事件计数器 + 可配置增益
;;
;; 演示内容：
;;   1 导出 alloc/dealloc 供宿主写入字符串与音频数据
;;   2 导入 micyou 模块的 host 函数（log / get_config / emit_event）
;;   3 process 处理实时音频（最佳努力，声明时不得声称 realtimeSafe）
;;   4 handle_event 统计事件次数并回传日志
;;
;; 构建（二选一）：
;;   a 安装 wabt：`wat2wasm counter.wat -o counter.wasm`
;;   b 使用 Rust：cargo install wasm-tools && wasm-tools parse counter.wat -o counter.wasm
;;
;; 安装：把 counter.wasm 与 plugin.json 放进
;;   ~/.config/micyou/plugins/dev.micyou.example.counter/

(module
  (import "micyou" "log" (func $log (param i32 i32)))
  (import "micyou" "get_config" (func $get_config (param i32) (result i32)))
  (import "micyou" "set_config" (func $set_config (param i32 i32) (result i32)))
  (import "micyou" "emit_event" (func $emit_event (param i32 i32) (result i32)))
  (import "micyou" "send_message" (func $send_message (param i32 i32 i32) (result i32)))
  (import "micyou" "audio_state" (func $audio_state (result i32)))
  (import "micyou" "connected_devices" (func $connected_devices (result i32)))

  (memory (export "memory") 1)

  ;; 静态数据区
  (data (i32.const 0) "counter events: \00")

  ;; 简单 bump 分配器（8 字节对齐）
  (global $bump (mut i32) (i32.const 1024))

  (func (export "alloc") (param $size i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (global.get $bump))
    (global.set $bump
      (i32.add
        (global.get $bump)
        (i32.and (i32.add (local.get $size) (i32.const 7)) (i32.const -8))))
    (local.get $ptr))

  (func (export "dealloc") (param $ptr i32) (param $size i32))

  (func (export "api_version") (result i32)
    (i32.const 1))

  (global $gain (mut f64) (f64.const 1.0))
  (global $events (mut i32) (i32.const 0))

  (func (export "init") (result i32)
    (call $log (i32.const 2) (i32.const 0)) ;; INFO "counter events: "
    (i32.const 0))

  ;; process(data_ptr, samples, channels, queued_ms) -> 0=ok 1=bypass
  (func (export "process")
    (param $ptr i32) (param $samples i32) (param $channels i32) (param $queued_ms f64)
    (result i32)
    (local $i i32) (local $gain_f32 f32)
    (local.set $gain_f32 (f32.demote_f64 (global.get $gain)))
    (if (f32.le (local.get $gain_f32) (f32.const 0.0))
      (then (return (i32.const 1))))
    (local.set $i (i32.const 0))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $samples)))
        (f32.store
          (i32.add (local.get $ptr) (i32.mul (local.get $i) (i32.const 4)))
          (f32.mul
            (f32.load (i32.add (local.get $ptr) (i32.mul (local.get $i) (i32.const 4))))
            (local.get $gain_f32)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (i32.const 0))

  ;; 事件计数器：每次收到事件 +1 并通过 emit_event 回传
  (func (export "handle_event") (param $json_ptr i32) (result i32)
    (global.set $events (i32.add (global.get $events) (i32.const 1)))
    (call $emit_event (i32.const 0) (i32.const 0)) ;; 忽略返回值，仅演示
    (i32.const 0))

  (func (export "handle_message") (param $ptr i32) (param $len i32) (result i32)
    (i32.const 0))

  (func (export "deinit"))
)
