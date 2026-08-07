# API 参考

插件系统的完整接口定义：Host API、Plugin API、消息协议 / 事件定义、错误码与权限清单

## Host API（宿主向插件提供的服务）

### Native（C ABI，`mpl_host_api_t`）

```c
typedef struct mpl_host_api {
    void (*log)(void *ctx, mpl_log_level_t level, const char *msg);
    mpl_result_t (*get_config)(void *ctx, const char *key, char *out, uint32_t *out_size);
    mpl_result_t (*set_config)(void *ctx, const char *key, const char *json_value);
    mpl_result_t (*emit_event)(void *ctx, const char *topic, const char *json_payload);
    mpl_result_t (*send_message)(void *ctx, const char *target_json, const uint8_t *payload, uint32_t payload_len);
    mpl_result_t (*audio_state)(void *ctx, char *out, uint32_t *out_size);
    mpl_result_t (*connected_devices)(void *ctx, char *out, uint32_t *out_size);
    void *ctx;
} mpl_host_api_t;
```

### WASM（导入模块 `micyou`）

| 导入 | 签名 | 说明 |
| --- | --- | --- |
| `log` | `(level: i32, msg_ptr: i32) -> ()` | level：0=Error 1=Warn 2=Info 3=Debug 4=Trace |
| `get_config` | `(key_ptr: i32) -> i32` | 返回宿主分配的 JSON 指针（0 = 无该键） |
| `set_config` | `(key_ptr: i32, value_json_ptr: i32) -> i32` | 返回结果码 |
| `emit_event` | `(topic_ptr: i32, payload_json_ptr: i32) -> i32` | 返回结果码 |
| `send_message` | `(target_json_ptr: i32, payload_ptr: i32, len: i32) -> i32` | 返回结果码 |
| `audio_state` | `() -> i32` | 返回宿主分配的 JSON 指针 |
| `connected_devices` | `() -> i32` | 返回宿主分配的 JSON 数组指针 |

### 缓冲区契约（out / out_size）

- `out` / `out_size` 描述插件提供的缓冲区（UTF-8）
- 成功：写入 NUL 结尾字符串，`*out_size` = 字节数（不含 NUL），返回 `MPL_OK`
- 缓冲区太小：`*out_size` = 所需大小，返回 `MPL_ERR_BUFFER_TOO_SMALL`
- `audio_state` 返回 JSON 快照：

```json
{ "streaming": true, "sampleRate": 48000, "channels": 1, "inputLevel": 0.42, "processedLevel": 0.38, "queuedMs": 12.5, "muted": false }
```

- `connected_devices` 返回 JSON 数组：

```json
[ { "mode": "wifi", "label": "MicYou Mobile", "audioActive": true } ]
```

## Plugin API（插件向宿主实现的接口）

### Native（C ABI 符号）

| 符号 | 必需 | 说明 |
| --- | --- | --- |
| `micyou_plugin_info` | 是 | 返回静态 `mpl_plugin_info_t`，含 ABI / API 版本与 id |
| `micyou_plugin_init(host)` | 是 | 保存 host 表，读取配置，返回结果码 |
| `micyou_plugin_deinit` | 是 | 清理（宿主卸载库前调用一次） |
| `micyou_plugin_process(data, samples, channels, queued_ms, bypass)` | 否 | 实时 DSP |
| `micyou_plugin_handle_event(type, json)` | 否 | 事件通知 |
| `micyou_plugin_handle_message(source, topic, payload, len)` | 否 | 跨端消息 |

### WASM（导出）

| 导出 | 必需 | 说明 |
| --- | --- | --- |
| `memory` | 是 | 线性内存 |
| `alloc` / `dealloc` | 是 | 内存分配（宿主写字符串 / 音频数据用） |
| `api_version` | 否 | 返回 Host API 版本（1） |
| `init` | 否 | 初始化，0 = 成功 |
| `process` | 否 | DSP，0 = ok 1 = bypass |
| `handle_event` | 否 | 事件（JSON 指针） |
| `handle_message` | 否 | 跨端消息（指针, 长度） |
| `deinit` | 否 | 反初始化 |

### 事件类型（`PluginEvent`）

| 事件 | 负载 |
| --- | --- |
| `device_connected` | `{ mode, label }` |
| `device_disconnected` | — |
| `mute_changed` | `{ muted }` |
| `dsp_settings_changed` | — |
| `state_changed` | `{ enabled }` |

## 消息协议

线协议为 protobuf（`crates/micyou-protocol/proto/network.proto`），挂载在控制通道 `MessageWrapper` 字段 7：

```proto
message PluginMessage {
    string source = 1;       // 发送方插件 id
    string target = 2;       // 接收方插件 id，"" = 广播
    string topic = 3;        // 主题
    bytes payload = 4;       // 插件自定义负载
    uint64 correlationId = 5;
    bool isResponse = 6;
    int32 errorCode = 7;     // 0 = ok
    string errorMessage = 8;
}
```

语义：

- **发布订阅**：`target` 为空，按 `topic` 分发（本地订阅者 + 远端广播）
- **请求响应**：`correlationId` 非 0 配对请求与响应；`isResponse` 标记响应
- **错误响应**：`errorCode` 非 0 + `errorMessage`

## 错误码

`PluginError` 与 wire 错误码的稳定映射（改变即破坏兼容）：

| 码 | 含义 |
| --- | --- |
| 0 | ok |
| 1 | not found（入口产物缺失） |
| 2 | invalid manifest |
| 3 | validation failed（清单语义校验） |
| 4 | unknown plugin |
| 5 | not loaded |
| 6 | load failed |
| 7 | api version mismatch |
| 8 | permission denied（能力未授予） |
| 9 | already exists |
| 10 | runtime error（含 WASM trap / 燃料耗尽） |
| 11 | message delivery failed（无设备 / 超时） |
| 12 | io error |

Native 的 `mpl_result_t` 数值与此保持一致（0-5 子集）

## 权限清单

| 能力 | 授予的 API | 风险 |
| --- | --- | --- |
| `dsp.node` | 处理链节点注册 | 实时音频数据访问 |
| `config.read` | get_config | 插件自身配置 |
| `config.write` | set_config | 插件自身配置 |
| `event.emit` | emit_event | 总线事件（含远端广播） |
| `message.send` | send_message | 跨端消息 |
| `audio.state` | audio_state | 音频流状态快照 |
| `device.list` | connected_devices | 已连接设备信息 |
| `network.io` | 预留 | 出站网络 |
| `fs.read` | 预留 | 沙箱内文件 |

- 未知能力声明会被清单校验拒绝
- 未声明能力的 API 调用被 host 回调层拒绝（`MPL_ERR_PERMISSION` / 错误码 8）
