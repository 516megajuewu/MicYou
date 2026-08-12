package com.lanrhyme.micyou.theme

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Shapes
import androidx.compose.ui.unit.dp

/**
 * Material 3 Expressive (2025) 形状系统
 * 更大的圆角半径，更有表现力的视觉风格
 */

/**
 * Expressive形状 - 标准Expressive风格圆角
 */
val ExpressiveShapes = Shapes(
    extraSmall = RoundedCornerShape(8.dp),   // M3标准: 4dp
    small = RoundedCornerShape(12.dp),       // M3标准: 8dp
    medium = RoundedCornerShape(20.dp),      // M3标准: 12dp
    large = RoundedCornerShape(28.dp),       // M3标准: 16dp
    extraLarge = RoundedCornerShape(40.dp)   // M3标准: 28dp
)
