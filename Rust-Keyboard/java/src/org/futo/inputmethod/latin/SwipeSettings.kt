package org.futo.inputmethod.latin

import androidx.datastore.preferences.core.booleanPreferencesKey
import org.futo.inputmethod.latin.uix.SettingsKey

// Swipe typing is not part of this build: the decoder, its models and the JNI
// library are gone. These settings survive because the keyboard and typing
// screens still read them, and they must keep their old keys so an existing
// install does not lose its values.
val LegacySwipeSetting = SettingsKey(booleanPreferencesKey("swipe_mode_legacy"), false)
val DisplayTop4Setting = SettingsKey(booleanPreferencesKey("swipe_use_top4_suggestions"), true)

val SwipeSpecialDecoderSetting = SettingsKey(booleanPreferencesKey("__experimental_swipe_special_decoder"), true)
val SwipeLanguageModelSetting = SettingsKey(booleanPreferencesKey("__experimental_swipe_language_model"), true)
