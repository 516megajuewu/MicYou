# MicYou 插件系统

MicYou 插件系统允许第三方为桌面端与（未来）安卓端扩展能力：实时 DSP 节点、工具逻辑、UI 面板与跨端同步桥

| 文档 | 说明 |
| --- | --- |
| [总览](overview.md) | 架构、双运行时、DSP 集成与跨端同步模型 |
| [开发指南](development-guide.md) | 编写 Native / WASM 插件、Manifest、Host API、实时安全规范 |
| [用户指南](user-guide.md) | 安装卸载、GUI 管理、配置与跨端同步使用 |
| [API 参考](api-reference.md) | Host API、Plugin API、消息协议、错误码与权限清单 |
| [架构与扩展](architecture-extensibility.md) | 安卓端扩展计划、版本兼容、安全模型 |

## 快速导航

- 想写插件：读 [开发指南](development-guide.md)
- 想装插件：读 [用户指南](user-guide.md)
- 想了解协议与权限：读 [API 参考](api-reference.md)
- 想知道安卓端怎么规划：读 [架构与扩展](architecture-extensibility.md)

## 示例插件

| 示例 | 运行时 | 类型 | 位置 |
| --- | --- | --- | --- |
| native-gain | Native (cdylib) | DSP 增益节点 | `plugins/examples/native-gain/` |
| wasm-counter | WASM | 事件计数 + 增益 | `plugins/examples/wasm-counter/` |

## 代码结构

```text
crates/micyou-plugin/            # 插件框架（桌面 + 未来安卓共用）
├── src/manifest.rs              # 统一清单模型与校验
├── src/plugin.rs                # 统一插件抽象（双运行时）
├── src/native.rs                # Native 加载器（libloading + C ABI）
├── src/wasm.rs                  # WASM 运行时（wasmi 沙箱 + 燃料计量）
├── src/abi.rs                   # C ABI host 回调桥
├── src/dsp.rs                   # DSP 节点注册表与链桥
├── src/bus.rs                   # 消息总线（发布订阅 / RPC）
├── src/sync.rs                  # 跨端线协议编解码
├── include/micyou_plugin_abi.h  # Native 插件 ABI 头文件（v1）
├── fixtures/                    # 测试夹具（native cdylib + wasm）
└── tests/                       # 集成测试
src-tauri/src/plugins.rs         # 桌面宿主接线（PluginHost）
src-tauri/src/commands/plugins.rs# 前端管理命令
src/features/plugins/            # 前端管理界面（Vue）
plugins/examples/                # 示例插件
docs/plugins/                    # 本文档
```

## 状态

- [x] 统一接口抽象 + PluginManager
- [x] Native 插件加载（C ABI v1）
- [x] WASM 插件运行时（wasmi）
- [x] DSP 链路集成（合成链节点 `Plugins`）
- [x] 跨端消息同步协议（protobuf `PluginMessage`）
- [x] 前端插件管理界面
- [x] 示例插件与文档
- [ ] 安卓端运行时（协议已就绪，见 [架构与扩展](architecture-extensibility.md)）
