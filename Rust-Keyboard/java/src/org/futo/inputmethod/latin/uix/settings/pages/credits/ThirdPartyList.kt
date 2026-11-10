package org.futo.inputmethod.latin.uix.settings.pages.credits


data class ThirdPartyItem(
    val name: String,
    val description: String,
    val projectUrl: String,
    val copyright: String,
    val license: License
)

typealias TP = ThirdPartyItem

@Suppress("HardCodedStringLiteral")
val ThirdPartyList: List<ThirdPartyItem> = listOf(
    TP(
        "AOSP LatinIME",
        "Keyboard based on AOSP LatinIME",
        "https://android.googlesource.com/platform/packages/inputmethods/LatinIME/",
        "Copyright (c) 2011 The Android Open Source Project",
        License.Apache2
    ),
    TP(
        "Feather Icons",
        "Feather Icons",
        "https://feathericons.com",
        "Copyright (c) 2013-2017 Cole Bemis",
        License.MIT,
    ),
    TP(
        "LineageOS LatinIME",
        "Some keyboard layouts used from LineageOS",
        "https://github.com/LineageOS/android_packages_inputmethods_LatinIME",
        "Copyright (c) 2015 The CyanogenMod Project",
        License.Apache2
    ),
    TP(
        "Reorderable",
        "Reorderable",
        "https://github.com/Calvin-LL/Reorderable",
        "Copyright (c) 2023 Calvin Liang",
        License.Apache2
    ),
    TP(
        "Noto Emoji",
        "Noto Emoji",
        "https://fonts.google.com/noto/specimen/Noto+Emoji",
        "Copyright (c) 2013 Google LLC",
        License.OFL1_1
    ),
    TP(
        "Noto Mono",
        "Noto Mono",
        "https://fonts.google.com/noto",
        "Copyright (c) 2022 The Noto Project Authors",
        License.OFL1_1
    ),
    TP(
        "Sour Gummy font",
        "Sour Gummy font - used in Christmas 2025 theme, modified so that the j doesn't look like an i",
        "https://github.com/eifetx/Sour-Gummy-Fonts",
        "Copyright 2018 The Sour Gummy Project Authors",
        License.OFL1_1
    ),
    TP(
        "Christmas 2025 theme",
        "Seasonal theme for Christmas 2025 (Gingerbread)",
        "https://zilluzion.art/",
        "Copyright (c) 2025 zilluzion",
        License.Proprietary
    ),
    TP(
        "ClearFlow",
        "ClearFlow Layout",
        "https://clearflowkeyboard.github.io/",
        "Copyright (c) 2024-2026 ClearFlow Authors (Xiaojun Bi et al.)",
        License.PublicDomainClearflow
    )
)
