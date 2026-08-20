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

package com.lanrhyme.micyou.audio

enum class SampleRate(val value: Int) {
    Rate16000(16000),
    Rate44100(44100),
    Rate48000(48000)
}

enum class ChannelCount(val value: Int, val label: String) {
    Mono(1, "Mono"),
    Stereo(2, "Stereo")
}

/**
 * 音频格式枚举
 * @param value 线上协议格式值（除 PCM_24BIT 的兼容配置值外，与 Android 编码值一致）
 * @param label 显示标签
 * @param bitsPerSample 每样本位数，用于计算比特率
 */
enum class AudioFormat(val value: Int, val label: String, val bitsPerSample: Int) {
    PCM_8BIT(3, "8-bit PCM", 8), // AudioFormat.ENCODING_PCM_8BIT = 3
    PCM_16BIT(2, "16-bit PCM", 16), // AudioFormat.ENCODING_PCM_16BIT = 2
    PCM_24BIT(6, "24-bit PCM", 24), // 保留配置兼容；AudioEngine 运行时安全回退到 PCM16
    PCM_FLOAT(4, "32-bit Float", 32) // AudioFormat.ENCODING_PCM_FLOAT = 4
}

/** AudioRecord 实际采集格式及对应的线上格式，必须作为同一个解析结果使用。 */
internal data class ResolvedAudioFormat(
    val captureFormat: AudioFormat,
    val androidEncoding: Int,
    val wireFormat: AudioFormat
) {
    val bytesPerSample: Int = captureFormat.bitsPerSample / Byte.SIZE_BITS
}

internal fun resolveAudioFormat(requestedFormat: AudioFormat): ResolvedAudioFormat {
    val resolvedFormat = when (requestedFormat) {
        AudioFormat.PCM_24BIT -> AudioFormat.PCM_16BIT
        else -> requestedFormat
    }
    return ResolvedAudioFormat(
        captureFormat = resolvedFormat,
        androidEncoding = resolvedFormat.value,
        wireFormat = resolvedFormat
    )
}
