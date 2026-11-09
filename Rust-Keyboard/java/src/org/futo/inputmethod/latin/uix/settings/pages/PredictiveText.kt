package org.futo.inputmethod.latin.uix.settings.pages

import android.content.Intent
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.booleanResource
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import org.futo.inputmethod.latin.R
import org.futo.inputmethod.latin.settings.Settings
import org.futo.inputmethod.latin.uix.USE_TRANSFORMER_FINETUNING
import org.futo.inputmethod.latin.uix.settings.NavigationItemStyle
import org.futo.inputmethod.latin.uix.settings.Tip
import org.futo.inputmethod.latin.uix.settings.UserSettingsMenu
import org.futo.inputmethod.latin.uix.settings.useSharedPrefsBool
import org.futo.inputmethod.latin.uix.settings.userSettingDecorationOnly
import org.futo.inputmethod.latin.uix.settings.userSettingNavigationItem
import org.futo.inputmethod.latin.uix.settings.userSettingToggleDataStore
import org.futo.inputmethod.latin.uix.settings.userSettingToggleSharedPrefs

// The on-device language model was removed: it was a multilingual model, so it
// predicted words in other languages, and it cost ~75 MB of package and
// hundreds of megabytes of RAM. Predictions now come from the English
// dictionary alone.
val PredictiveTextMenu = UserSettingsMenu(
    title = R.string.prediction_settings_title,
    navPath = "predictiveText", registerNavPath = true,
    settings = listOf(
        userSettingNavigationItem(
            title = R.string.edit_personal_dictionary,
            style = NavigationItemStyle.HomePrimary,
            icon = R.drawable.book,
            navigate = { nav ->
                nav.navigate("pdict")
            }
        ),

        userSettingNavigationItem(
            title = R.string.prediction_settings_word_blacklist,
            style = NavigationItemStyle.HomeSecondary,
            icon = R.drawable.file_text,
            navigateTo = "blacklist"
        ),

        // TODO: It doesn't make a lot of sense in the case of having autocorrect on but show_suggestions off
        userSettingToggleSharedPrefs(
            title = R.string.auto_correction,
            subtitle = R.string.auto_correction_summary,
            key = Settings.PREF_AUTO_CORRECTION,
            default = {true},
            icon = {
                Icon(painterResource(id = R.drawable.icon_spellcheck), contentDescription = null,
                    tint = MaterialTheme.colorScheme.onBackground.copy(alpha = 0.75f))
            }
        ).copy(searchTags = R.string.auto_correction_tags),

        userSettingToggleSharedPrefs(
            title = R.string.prediction_settings_smart_keyhit_detection,
            subtitle = R.string.prediction_settings_smart_keyhit_detection_subtitle,
            key = Settings.PREF_USE_DICT_KEY_BOOSTING,
            default = {true}
        ),

        userSettingToggleSharedPrefs(
            title = R.string.prefs_show_suggestions,
            subtitle = R.string.prefs_show_suggestions_summary,
            key = Settings.PREF_SHOW_SUGGESTIONS,
            default = {true}
        ),

        userSettingToggleSharedPrefs(
            title = R.string.use_personalized_dicts,
            subtitle = R.string.use_personalized_dicts_summary,
            key = Settings.PREF_KEY_USE_PERSONALIZED_DICTS,
            default = {true}
        ),

        userSettingToggleSharedPrefs(
            title = R.string.bigram_prediction,
            subtitle = R.string.bigram_prediction_summary,
            key = Settings.PREF_BIGRAM_PREDICTIONS,
            default = { booleanResource(R.bool.config_default_next_word_prediction) }
        )
    )
)