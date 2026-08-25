/*
 * MicYou — Turns your Android device into a high-quality PC microphone.
 * Copyright (C) 2026 LanRhyme <https://github.com/LanRhyme/MicYou>
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version, with the MicYou Plugin Exception.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU General Public License for more details.
 */

package com.lanrhyme.micyou.viewmodel

import androidx.lifecycle.ViewModel
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeout
import com.lanrhyme.micyou.audio.AudioEngine
import com.lanrhyme.micyou.audio.AudioFormat
import com.lanrhyme.micyou.audio.ChannelCount
import com.lanrhyme.micyou.audio.SampleRate
import com.lanrhyme.micyou.network.calculateUdpPort
import com.lanrhyme.micyou.network.ConnectionErrorDetails
import com.lanrhyme.micyou.network.ConnectionErrorHelper
import com.lanrhyme.micyou.network.DeviceDiscoveryManager
import com.lanrhyme.micyou.network.DiscoveredDevice
import com.lanrhyme.micyou.settings.Settings
import com.lanrhyme.micyou.settings.SettingsFactory
import com.lanrhyme.micyou.util.AppLanguage
import com.lanrhyme.micyou.util.Constants
import com.lanrhyme.micyou.util.ContextHelper
import com.lanrhyme.micyou.util.Logger
import com.lanrhyme.micyou.viewmodel.AudioStreamUiState
import java.util.concurrent.atomic.AtomicBoolean

data class AudioStreamUiState(
    val mode: ConnectionMode = ConnectionMode.Wifi,
    val transportProtocol: TransportProtocol = TransportProtocol.Both,
    val streamState: StreamState = StreamState.Idle,
    /** 目标服务端 IP；空表示尚未配置（未手动输入且未发现服务端），此时禁止自动连接 */
    val ipAddress: String = "",
    val port: String = Constants.DEFAULT_TCP_PORT.toString(),
    val errorMessage: String? = null,
    val sampleRate: SampleRate = SampleRate.Rate48000,
    val channelCount: ChannelCount = ChannelCount.Stereo,
    val audioFormat: AudioFormat = AudioFormat.PCM_FLOAT,
    val isMuted: Boolean = false,
    val isAutoConfig: Boolean = true,
    val autoReconnect: Boolean = true,
    /** 下次自动重连的时间点（epoch ms）；null 表示当前没有排期的重连 */
    val nextReconnectAtMillis: Long? = null,
    /** 即将执行的自动重连尝试次数（从 1 开始） */
    val reconnectAttempt: Int = 0,
    // Error Dialog State
    val showErrorDialog: Boolean = false,
    val errorDetails: ConnectionErrorDetails? = null,

    // UDP Warning Dialog State
    val showUdpWarningDialog: Boolean = false,

    val androidAudioSourceName: String = "Mic"
)

class AudioStreamViewModel : ViewModel() {
    private val _audioEngine = AudioEngine()
    val audioEngine: AudioEngine get() = _audioEngine
    private val auxiliaryScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val closed = AtomicBoolean(false)
    private val closeLock = Any()
    private var closeJob: Job? = null
    private val _uiState = MutableStateFlow(AudioStreamUiState())
    val uiState: StateFlow<AudioStreamUiState> = _uiState.asStateFlow()

    // 音频电平相关
    val audioLevels = _audioEngine.audioLevels

    // 设备发现
    private val discoveryManager = DeviceDiscoveryManager()
    val discoveredDevices: StateFlow<List<DiscoveredDevice>> = discoveryManager.discoveredDevices
    val isDiscovering: StateFlow<Boolean> = discoveryManager.isDiscovering

    private val settings = SettingsFactory.getSettings()
    private var isStartStreamRequestPending = false
    private var isStopStreamRequestPending = false

    /**
     * 用户意图：用户最后一次明确想要流继续运行的标志。
     * 注意不能依赖引擎内部 desiredRunning —— 引擎在 stop 超时等异常路径会把
     * desiredRunning 置 false 且无人复位，若重连门槛依赖它会导致重连链一次
     * 性断裂（表现：UI 停在"未知网络错误"，只有手动点才能恢复）。
     */
    @Volatile
    private var userWantsRunning = false

    /**
     * 用户已请求连接但目标地址尚未配置：等待 mDNS 发现服务端后自动接续连接。
     * 避免在没有任何服务端信息时盲连一个默认 IP。
     */
    @Volatile
    private var pendingAutoConnect = false

    init {
        loadSettings()
        setupAudioEngineObservers()
        registerNetworkCallback()
        if (_uiState.value.mode == ConnectionMode.Wifi) {
            discoveryManager.startDiscovery()
        }
    }

    private fun loadSettings() {
        val savedModeName = settings.getString("connection_mode", ConnectionMode.Wifi.name)
    val savedMode = when (savedModeName) {
            "WifiUdp" -> ConnectionMode.Wifi
            else -> try { ConnectionMode.valueOf(savedModeName) } catch(e: Exception) { ConnectionMode.Wifi }
        }
        val effectiveMode = savedMode
        val savedProtocolName = settings.getString("transport_protocol", TransportProtocol.Both.name)
        val savedProtocol = try { TransportProtocol.valueOf(savedProtocolName) } catch(e: Exception) { TransportProtocol.Both }
    val savedIp = settings.getString("ip_address", "")
    val savedPort = settings.getString("port", Constants.DEFAULT_TCP_PORT.toString())
    val savedSampleRateName = settings.getString("sample_rate", SampleRate.Rate48000.name)
    val savedSampleRate = try { SampleRate.valueOf(savedSampleRateName) } catch(e: Exception) { SampleRate.Rate48000 }
    val savedChannelCountName = settings.getString("channel_count", ChannelCount.Stereo.name)
    val savedChannelCount = try { ChannelCount.valueOf(savedChannelCountName) } catch(e: Exception) { ChannelCount.Stereo }
    val savedAudioFormatName = settings.getString("audio_format", AudioFormat.PCM_FLOAT.name)
    val savedAudioFormat = try { AudioFormat.valueOf(savedAudioFormatName) } catch(e: Exception) { AudioFormat.PCM_FLOAT }

    val savedAndroidAudioSourceName = settings.getString("android_audio_source", "Mic")
    val savedIsAutoConfig = settings.getBoolean("is_auto_config", true)
    val savedAutoReconnect = settings.getBoolean("auto_reconnect", true)

        _uiState.update {
            it.copy(
                mode = effectiveMode,
                transportProtocol = savedProtocol,
                ipAddress = savedIp,
                port = savedPort,
                sampleRate = savedSampleRate,
                channelCount = savedChannelCount,
                audioFormat = savedAudioFormat,
                androidAudioSourceName = savedAndroidAudioSourceName,
                isAutoConfig = savedIsAutoConfig,
                autoReconnect = savedAutoReconnect
            )
        }

        // Apply auto config on startup if enabled
        if (savedIsAutoConfig) {
            applyAutoConfig()
        }
    }

    private fun setupAudioEngineObservers() {
        auxiliaryScope.launch {
            _audioEngine.streamState.collect { state ->
                _uiState.update { it.copy(streamState = state) }
                when (state) {
                    StreamState.Streaming -> resetAutoReconnect()
                    StreamState.Connecting -> lastConnectingAtMillis = System.currentTimeMillis()
                    StreamState.Error -> scheduleAutoReconnect()
                    else -> {}
                }
            }
        }

        // Connecting 卡死看门狗：任何未知路径导致连接建立阶段长时间无事件时，
        // 强制清理僵死会话并恢复自动重连链路（正常路径由 15s 总超时兜底，
        // 看门狗是最后一道防线，阈值大于总超时避免误伤）。
        auxiliaryScope.launch {
            while (true) {
                delay(1000)
                if (_uiState.value.streamState != StreamState.Connecting) continue
                val waitedMs = System.currentTimeMillis() - lastConnectingAtMillis
                if (waitedMs < CONNECTING_WATCHDOG_TIMEOUT_MS) continue
                Logger.w("AudioStreamViewModel", "Connecting stuck for ${waitedMs}ms, watchdog forcing recovery")
                try {
                    // 非用户主动停止：保留 desiredRunning，仅清理会话资源
                    _audioEngine.stopAndWait(userInitiated = false)
                } catch (e: Exception) {
                    Logger.w("AudioStreamViewModel", "Watchdog stop failed: ${e.message}")
                }
                if (!canAutoReconnect()) continue
                _uiState.update { it.copy(streamState = StreamState.Error) }
                scheduleAutoReconnect()
            }
        }

        auxiliaryScope.launch {
            _audioEngine.lastError.collect { error ->
                if (error == "UDP_AUDIO_WARNING") {
                    _uiState.update { it.copy(showUdpWarningDialog = true) }
                } else {
                    _uiState.update { it.copy(errorMessage = error) }
                }
            }
        }

        auxiliaryScope.launch {
            _audioEngine.isMuted.collect { muted ->
                _uiState.update { it.copy(isMuted = muted) }
            }
        }

        // 服务端自动发现：
        // 1) 尚未配置地址时（首次安装）自动填入并接续连接，避免盲连默认 IP；
        // 2) 已配置但重连连续失败时，若发现的服务端地址与当前不同（如路由器
        //    重启后 DHCP 换了 PC 的 IP），自动切换到新地址，否则会永远重连到
        //    一个已失效的地址。
        auxiliaryScope.launch {
            discoveryManager.discoveredDevices.collect { devices ->
                val device = devices.firstOrNull() ?: return@collect
                val current = _uiState.value
                val addressMissing = current.ipAddress.isBlank()
                val addressStale = !addressMissing &&
                    current.streamState == StreamState.Error &&
                    reconnectAttempts >= ADOPT_DISCOVERED_AFTER_ATTEMPTS &&
                    device.hostAddress != current.ipAddress
                if (!addressMissing && !addressStale) return@collect

                Logger.i(
                    "AudioStreamViewModel",
                    "Adopting discovered server ${device.hostAddress}:${device.port} (missing=$addressMissing, stale=$addressStale)"
                )
                setIp(device.hostAddress)
                setPort(device.port.toString())

                if (pendingAutoConnect || userWantsRunning) {
                    pendingAutoConnect = false
                    // 地址已更新：取消排期中的旧重连，立即用新地址尝试
                    cancelAutoReconnect()
                    startStream()
                }
            }
        }

        // Auto-start handled via MainViewModel
    }

    private fun applyAutoConfig() {
        setSampleRate(SampleRate.Rate48000)
        setChannelCount(ChannelCount.Stereo)
        setAudioFormat(AudioFormat.PCM_16BIT)
    }

    fun toggleStream() {
        if (_uiState.value.streamState == StreamState.Streaming || _uiState.value.streamState == StreamState.Connecting) {
            stopStream()
        } else {
            startStream()
        }
    }

    fun toggleMute() {
        val newMuteState = !_uiState.value.isMuted
        auxiliaryScope.launch {
            _audioEngine.setMute(newMuteState)
        }
    }

    fun startStream() {
        if (isStartStreamRequestPending ||
            isStopStreamRequestPending ||
            _uiState.value.streamState == StreamState.Streaming ||
            _uiState.value.streamState == StreamState.Connecting
        ) {
            Logger.d("AudioStreamViewModel", "Start stream request ignored: already starting or running")
            return
        }

        isStartStreamRequestPending = true
        userWantsRunning = true
        // 手动发起的启动：取消排期中的自动重连，避免与本次启动并发执行
        cancelAutoReconnect()

        // 目标地址尚未配置（首次安装且未扫描到服务端）：不盲连默认 IP，
        // 改为登记待连接意图，等 mDNS 发现服务端后由发现观察者接续启动。
        if (_uiState.value.mode == ConnectionMode.Wifi && _uiState.value.ipAddress.isBlank()) {
            Logger.i("AudioStreamViewModel", "No target address yet, waiting for discovery")
            pendingAutoConnect = true
            isStartStreamRequestPending = false
            discoveryManager.startDiscovery()
            return
        }

        auxiliaryScope.launch {
            try {
                startStreamInternal()
            } finally {
                isStartStreamRequestPending = false
            }
        }
    }

    private suspend fun startStreamInternal(fromReconnect: Boolean = false) {
        Logger.i("AudioStreamViewModel", "Starting stream")
        val mode = _uiState.value.mode
        val ip = _uiState.value.ipAddress

        // 端口验证：确保端口在有效范围内 (1-65535)
        val rawPort = _uiState.value.port.toIntOrNull()
        val port = when {
            rawPort == null -> {
                Logger.w("AudioStreamViewModel", "Invalid port format: ${_uiState.value.port}, using default ${Constants.DEFAULT_TCP_PORT}")
                Constants.DEFAULT_TCP_PORT
            }
            rawPort <= 0 || rawPort > 65535 -> {
                Logger.w("AudioStreamViewModel", "Port out of range: $rawPort, using default ${Constants.DEFAULT_TCP_PORT}")
                Constants.DEFAULT_TCP_PORT
            }
            else -> rawPort
        }

        // IP 地址验证（USB 模式由引擎强制走 127.0.0.1，无需地址）
        if (ip.isBlank() && mode != ConnectionMode.Usb) {
                Logger.e("AudioStreamViewModel", "IP address is empty")
                // 不弹对话框：地址缺失时登记待连接意图，等发现服务端后自动接续
                pendingAutoConnect = true
                discoveryManager.startDiscovery()
                _uiState.update {
                    it.copy(
                        streamState = StreamState.Idle,
                        errorMessage = null,
                        showErrorDialog = false,
                        errorDetails = null
                    )
                }
                return
            }
            // 基本的 IP 格式验证
            val ipRegex = Regex("^((25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\\.){3}(25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)$")
            if (!ipRegex.matches(ip) && !ip.startsWith("127.")) {
                Logger.w("AudioStreamViewModel", "IP address format may be invalid: $ip")
            }

        val sampleRate = _uiState.value.sampleRate
        val channelCount = _uiState.value.channelCount
        val audioFormat = _uiState.value.audioFormat

        _uiState.update { it.copy(streamState = StreamState.Connecting, errorMessage = null, showErrorDialog = false, errorDetails = null) }
        // 从尝试起点刷新看门狗计时，避免依赖引擎事件的到达时机造成误判
        lastConnectingAtMillis = System.currentTimeMillis()

        try {
            Logger.d("AudioStreamViewModel", "Calling _audioEngine.start()")
            try {
                // 连接建立阶段兜底超时：TCP connect/握手在网络不可达（如 Wi-Fi 断网/重开、
                // 目标 IP 失效）时可能长时间挂起。必须在 UI 层限制总时长，保证失败后进入
                // 退避重连，避免界面长期停留在 Connecting。
                withTimeout(STREAM_START_TIMEOUT_MS) {
                    _audioEngine.start(ip, port, mode, true, sampleRate, channelCount, audioFormat, _uiState.value.transportProtocol)
                }
            } catch (e: kotlinx.coroutines.TimeoutCancellationException) {
                // 转成普通超时异常，复用下方 catch(Exception) 的错误分析/本地化路径
                throw java.net.SocketTimeoutException("Stream start timed out after ${STREAM_START_TIMEOUT_MS}ms")
            }
            Logger.i("AudioStreamViewModel", "Stream started successfully")
        } catch (e: kotlinx.coroutines.CancellationException) {
            Logger.i("AudioStreamViewModel", "Stream start cancelled by user")
            // 自动重连尝试被取消/取代（上层竞态）时，若不恢复调度，UI 会卡在 Connecting
            // 且不再有任何事件触发重连。这里显式恢复：状态回置 Error 并按退避继续。
            if (fromReconnect && canAutoReconnect()) {
                _uiState.update { it.copy(streamState = StreamState.Error) }
                scheduleAutoReconnect()
            }
            return
        } catch (e: Exception) {
            Logger.e("AudioStreamViewModel", "Failed to start stream", e)

            val errorType = ConnectionErrorHelper.analyzeError(e, mode)
            val savedLanguageName = settings.getString("language", AppLanguage.System.name)
            val language = try {
                AppLanguage.valueOf(savedLanguageName)
            } catch (ex: Exception) {
                AppLanguage.System
            }
            val errorDetails = ConnectionErrorHelper.generateErrorDetails(
                type = errorType,
                originalMessage = e.message ?: "Unknown error",
                mode = mode,
                port = port,
                ip = ip
            )

            _uiState.update {
                it.copy(
                    streamState = StreamState.Error,
                    errorMessage = errorDetails.localizedMessage,
                    // 自动重连开启时一律不弹模态对话框，错误信息由横幅 + 倒计时呈现
                    showErrorDialog = !fromReconnect && !it.autoReconnect,
                    errorDetails = if (fromReconnect || it.autoReconnect) null else errorDetails
                )
            }
            // 失败后保证进入退避重连链：手动/自动路径都可能走到这里。
            // 若当前正是重连 job 内部（isActive），由 job 内部的后续推进兜底；
            // 外部入口的幂等检查保证不重复创建。若 isActive 已结束（比如重连
            // job 因取消而跳过推进），这里必须补一次调度，防止中断。
            if (fromReconnect) {
                if (reconnectJob?.isActive != true) {
                    scheduleNextReconnect()
                }
            } else {
                scheduleNextReconnect()
            }
            return
        }
    }

    fun stopStream() {
        Logger.i("AudioStreamViewModel", "Stopping stream")
        // 用户主动停止：取消挂起的自动重连，避免停止后被意外重连
        userWantsRunning = false
        pendingAutoConnect = false
        resetAutoReconnect()
        if (isStopStreamRequestPending) {
            Logger.d("AudioStreamViewModel", "Stop stream request ignored: stop already pending")
            return
        }
        isStopStreamRequestPending = true
        auxiliaryScope.launch {
            try {
                _audioEngine.stopAndWait()
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                Logger.e("AudioStreamViewModel", "Failed to stop stream", e)
            } finally {
                isStopStreamRequestPending = false
            }
        }
    }

    fun setMode(mode: ConnectionMode) {
        Logger.i("AudioStreamViewModel", "Setting connection mode to $mode")

        val current = _uiState.value

        val updatedPort = when (mode) {
            ConnectionMode.Usb -> {
                val parsed = current.port.toIntOrNull()
                if (parsed == null || parsed <= 0) Constants.DEFAULT_TCP_PORT.toString() else current.port
            }
            else -> current.port
        }

        // Auto-configure if enabled
        if (current.isAutoConfig) {
            applyAutoConfig()
        }

        _uiState.update { it.copy(mode = mode, port = updatedPort) }
        settings.putString("connection_mode", mode.name)

        // Manage discovery lifecycle based on mode
        if (mode == ConnectionMode.Wifi) {
            discoveryManager.startDiscovery()
        } else {
            discoveryManager.stopDiscovery()
        }
        if (updatedPort != current.port) {
            settings.putString("port", updatedPort)
        }
    }

    fun setTransportProtocol(protocol: TransportProtocol) {
        Logger.i("AudioStreamViewModel", "Setting transport protocol to $protocol")
        _uiState.update { it.copy(transportProtocol = protocol) }
        settings.putString("transport_protocol", protocol.name)
    }

    fun setIp(ip: String, restartStream: Boolean = false) {
        Logger.d("AudioStreamViewModel", "Setting IP to $ip, restartStream=$restartStream")
        val wasRunning = _uiState.value.streamState == StreamState.Streaming || _uiState.value.streamState == StreamState.Connecting

        _uiState.update {
            it.copy(
                ipAddress = ip.ifBlank { _uiState.value.ipAddress }
            )
        }
        settings.putString("ip_address", ip.ifBlank { _uiState.value.ipAddress })

    // 若要求重启流（IP 切换时），先停止再启动
        if (restartStream && wasRunning) {
            resetAutoReconnect()
            auxiliaryScope.launch(Dispatchers.IO) {
                try {
                    _audioEngine.stopAndWait()
                    startStreamInternal()
                } catch (e: Exception) {
                    Logger.e("AudioStreamViewModel", "Failed to restart stream after IP change", e)
                }
            }
        }
    }

    fun setPort(port: String) {
        // 允许空字符串，以便用户重新输入
        if (port.isBlank()) {
            _uiState.update { it.copy(port = "") }
            return
        }

        // 验证端口输入是否为数字且在有效范围内
        val portInt = port.toIntOrNull()
        if (portInt != null && portInt in 1..65535) {
            Logger.d("AudioStreamViewModel", "Setting port to $port")
            _uiState.update { it.copy(port = port) }
            settings.putString("port", port)
        } else {
            Logger.d("AudioStreamViewModel", "Invalid port input ignored: $port")
            // 如果是非数字字符，我们不更新状态，保持原样
        }
    }

    fun setSampleRate(rate: SampleRate) {
        _uiState.update { it.copy(sampleRate = rate) }
        settings.putString("sample_rate", rate.name)
    }

    fun setChannelCount(count: ChannelCount) {
        _uiState.update { it.copy(channelCount = count) }
        settings.putString("channel_count", count.name)
    }

    fun setAudioFormat(format: AudioFormat) {
        _uiState.update { it.copy(audioFormat = format) }
        settings.putString("audio_format", format.name)
    }

    fun setAndroidAudioSource(sourceName: String) {
        _uiState.update { it.copy(androidAudioSourceName = sourceName) }
        settings.putString("android_audio_source", sourceName)
        _audioEngine.setAudioSource(sourceName)
    }

    fun setAutoConfig(enabled: Boolean) {
        _uiState.update { it.copy(isAutoConfig = enabled) }
        settings.putBoolean("is_auto_config", enabled)
        if (enabled) {
            applyAutoConfig()
        }
    }

    fun dismissErrorDialog() {
        _uiState.update { it.copy(showErrorDialog = false) }
    }

    fun dismissUdpWarningDialog() {
        _uiState.update { it.copy(showUdpWarningDialog = false) }
    }

    fun retryAfterError() {
        dismissErrorDialog()
        // 手动重试：取消退避中的自动重连，立即重试
        resetAutoReconnect()
        startStream()
    }

    // ===== 自动重连 =====

    private companion object {
        /** 首次重连延迟 */
        const val RECONNECT_INITIAL_DELAY_MS = 3000L
        /** 重连延迟上限（指数退避封顶） */
        const val RECONNECT_MAX_DELAY_MS = 30000L
        /** 指数退避指数上限，避免位移溢出 */
        const val RECONNECT_MAX_BACKOFF_EXP = 10
        /** 连接建立总超时（TCP connect + 握手）；网络不可达时兜底失败进入退避，不卡 Connecting */
        const val STREAM_START_TIMEOUT_MS = 15000L
        /** Connecting 卡死看门狗阈值：大于总超时，仅拦截未知路径的僵死会话 */
        const val CONNECTING_WATCHDOG_TIMEOUT_MS = 20000L
        /** 连续重连失败达到该次数后，允许采用 mDNS 发现到的新地址（应对 DHCP 换 IP） */
        const val ADOPT_DISCOVERED_AFTER_ATTEMPTS = 2
    }

    private var reconnectJob: Job? = null
    private var reconnectAttempts = 0
    private var lastConnectingAtMillis = 0L

    /**
     * 重连代数：每次创建新一轮重连递增。被取消/取代的旧 job 通过
     * "自身的 gen != 当前 gen"判断自身已失效，在其后无论失败还是被取消，
     * 都不会再推进链条或与新一轮交错，从根上杜绝并发双会话竞态。
     */
    private var reconnectGeneration = 0L

    fun setAutoReconnect(enabled: Boolean) {
        Logger.i("AudioStreamViewModel", "Setting auto reconnect to $enabled")
        _uiState.update { it.copy(autoReconnect = enabled) }
        settings.putBoolean("auto_reconnect", enabled)
        if (!enabled) {
            resetAutoReconnect()
        }
    }

    /**
     * 断连后由 streamState 观察者触发：按退避策略（3s 起、指数增长、30s 封顶）
     * 调度一次重连尝试；失败后再次进入 Error 状态会继续下一轮退避。
     * 外部入口幂等：已有排期/进行中的重连时直接返回，避免观察者与显式调用
     * 双路径重复调度。重连 job 内部执行完会自行推进下一轮（见 scheduleNextReconnect）。
     */
    private fun scheduleAutoReconnect() {
        if (!canAutoReconnect()) return
        if (reconnectJob?.isActive == true) return
        scheduleNextReconnect()
    }

    /**
     * 创建/推进下一轮重连（不经幂等检查）。
     * 必须由重连 job 内部调用以保证"上一轮失败后链条不中断"：
     * 若失败时依赖外部入口重排，会因幂等检查（执行中 isActive=true）被拒绝，
     * 且 job 结束后引擎状态已是 Error 无新事件，观察者不会再来触发 —— 链条会
     * 永久断裂，表现为"连接超时横幅 + 倒计时消失 + 不再重连"。
     */
    private fun scheduleNextReconnect() {
        reconnectGeneration++
        val generation = reconnectGeneration
        val attempt = reconnectAttempts + 1
        reconnectAttempts = attempt
        val backoffExp = (attempt - 1).coerceAtMost(RECONNECT_MAX_BACKOFF_EXP)
        val delayMs = minOf(RECONNECT_INITIAL_DELAY_MS shl backoffExp, RECONNECT_MAX_DELAY_MS)
        Logger.i("AudioStreamViewModel", "Auto-reconnect attempt $attempt scheduled in ${delayMs}ms (gen=$generation)")
        _uiState.update {
            it.copy(
                nextReconnectAtMillis = System.currentTimeMillis() + delayMs,
                reconnectAttempt = attempt
            )
        }
        reconnectJob = auxiliaryScope.launch {
            delay(delayMs)
            // 若已被取消/取代（新的一轮已经创建），本 job 直接失效退出
            if (generation != reconnectGeneration) return@launch
            // 等待期间可能已被用户停止/重试/关闭；状态守卫放宽到 Idle：
            // 引擎清理路径可能把状态落成 Idle（而非 Error），此时同样应继续重连
            if (!canAutoReconnect()) return@launch
            val currentState = _uiState.value.streamState
            if (currentState != StreamState.Error && currentState != StreamState.Idle) return@launch
            // 网络恢复后 mDNS 发现可能已被系统中断，重启发现以便感知服务端新地址
            if (_uiState.value.mode == ConnectionMode.Wifi) {
                discoveryManager.startDiscovery()
            }
            Logger.i("AudioStreamViewModel", "Auto-reconnect attempt $attempt executing (gen=$generation)")
            startStreamInternal(fromReconnect = true)
            // 核心修复：本 job 已执行完毕，无论成功与否都在此推进下一轮，
            // 不依赖外部入口（其幂等检查会在此处被误伤）。gen 守卫保证
            // 被取消/取代的旧 job 不会与新一代交错推进。
            if (generation == reconnectGeneration &&
                _uiState.value.streamState != StreamState.Streaming && canAutoReconnect()
            ) {
                scheduleNextReconnect()
            }
        }
    }

    /**
     * 仅当自动重连开启、用户意图仍为运行（未被主动停止/关闭）、
     * 且 ViewModel 尚未关闭时才允许继续重连。
     * 用户意图由 ViewModel 自持（userWantsRunning），不依赖引擎内部状态，
     * 避免引擎异常路径的副作用（如 stop 超时置 desiredRunning=false）掐断重连。
     */
    private fun canAutoReconnect(): Boolean =
        !closed.get() && _uiState.value.autoReconnect && userWantsRunning

    private fun cancelAutoReconnect() {
        // 递增代数使所有已排期/进行中的重连 job 立即失效，
        // 即使它们正在执行或即将完成，也不会再推进链条
        reconnectGeneration++
        reconnectJob?.cancel()
        reconnectJob = null
    }

    /** 取消挂起的重连并清零重试计数（流成功后/主动停止时调用） */
    private fun resetAutoReconnect() {
        reconnectAttempts = 0
        cancelAutoReconnect()
        _uiState.update { it.copy(nextReconnectAtMillis = null, reconnectAttempt = 0) }
    }

    fun close(): Job = synchronized(closeLock) {
        closeJob?.let { return@synchronized it }
        closed.set(true)
        userWantsRunning = false
        pendingAutoConnect = false
        unregisterNetworkCallback()
        resetAutoReconnect()
        discoveryManager.stopDiscovery()
        val engineCloseJob = _audioEngine.close()
        auxiliaryScope.cancel()
        closeJob = engineCloseJob
        engineCloseJob
    }

    override fun onCleared() {
        close()
        super.onCleared()
    }

    fun startDiscovery() {
        discoveryManager.startDiscovery()
    }
    fun stopDiscovery() {
        discoveryManager.stopDiscovery()
    }
    fun restartDiscovery() {
        discoveryManager.stopDiscovery()
        discoveryManager.startDiscovery()
    }

    // ===== 网络恢复监听 =====
    // 快速开关 WiFi 场景下，网络恢复的瞬间立即触发一次连接尝试，
    // 避免卡在退避等待（可长达 30s），提高重连体验。

    private var networkCallback: android.net.ConnectivityManager.NetworkCallback? = null

    private fun registerNetworkCallback() {
        try {
            val context = ContextHelper.getContext() ?: return
            val cm = context.getSystemService(android.content.Context.CONNECTIVITY_SERVICE)
                as? android.net.ConnectivityManager ?: return
            val mainHandler = android.os.Handler(android.os.Looper.getMainLooper())
            networkCallback = object : android.net.ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: android.net.Network) {
                    mainHandler.post {
                        if (closed.get()) return@post
                        val caps = cm.getNetworkCapabilities(network)
                        val isWifi = caps?.hasTransport(android.net.NetworkCapabilities.TRANSPORT_WIFI) == true
                        if (!isWifi) return@post
                        val state = _uiState.value.streamState
                        if ((state == StreamState.Error || state == StreamState.Idle) && canAutoReconnect()) {
                            Logger.i("AudioStreamViewModel", "Network available, triggering immediate reconnect")
                            resetAutoReconnect()
                            startStream()
                        }
                    }
                }
            }
            cm.registerDefaultNetworkCallback(networkCallback!!)
            Logger.i("AudioStreamViewModel", "Network availability callback registered")
        } catch (e: Exception) {
            Logger.w("AudioStreamViewModel", "Failed to register network callback: ${e.message}")
            networkCallback = null
        }
    }

    private fun unregisterNetworkCallback() {
        try {
            val context = ContextHelper.getContext() ?: return
            val cm = context.getSystemService(android.content.Context.CONNECTIVITY_SERVICE)
                as? android.net.ConnectivityManager ?: return
            networkCallback?.let { cm.unregisterNetworkCallback(it) }
        } catch (e: Exception) {
            Logger.w("AudioStreamViewModel", "Failed to unregister network callback: ${e.message}")
        }
        networkCallback = null
    }

}
