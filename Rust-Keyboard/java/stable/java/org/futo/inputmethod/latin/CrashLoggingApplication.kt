package org.futo.inputmethod.latin

import android.app.Application
import android.content.Context
import android.os.UserManager
import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalContext
import androidx.datastore.preferences.core.Preferences
//import androidx.work.Configuration
import androidx.core.content.edit
import org.futo.inputmethod.latin.settings.Settings
import org.futo.inputmethod.latin.uix.isDirectBootUnlocked
import org.futo.inputmethod.latin.uix.settings.LocalDataStoreCache
import org.futo.inputmethod.latin.uix.settings.NavigationItem
import org.futo.inputmethod.latin.uix.settings.NavigationItemStyle
import org.futo.inputmethod.latin.uix.settings.pages.copyToClipboard
import kotlin.collections.plus

class CrashLoggingApplication : Application() /*, Configuration.Provider*/ {
    //override val workManagerConfiguration: Configuration
    //    get() = Configuration.Builder().build()

    override fun attachBaseContext(base: Context?) {
        super.attachBaseContext(base)

        if(isDirectBootUnlocked) {
            try {
                if (getSharedPreferences("migrate", MODE_PRIVATE).getBoolean(
                        "wiped_work",
                        false
                    ) == false
                ) {
                    deleteDatabase("androidx.work.workdb")
                    getSharedPreferences("androidx.work.util.preferences", MODE_PRIVATE)
                        .edit { clear() }
                    getSharedPreferences("migrate", MODE_PRIVATE)
                        .edit { putBoolean("wiped_work", true) }
                }
            } catch(e: Exception) {
                e.printStackTrace()
            }
        }

        // Crash reporting has been removed: nothing is collected, stored for
        // upload, or sent anywhere. The crash dialog that ACRA used to show is
        // gone with it. "Copy logs" in the settings still works and stays local.
        if (BuildConfig.DEBUG) return
    }

    companion object {
        // ACRA is not part of this build.
        var acraInitialized = false

        fun logPreferences(preferences: Preferences) {
            // No crash reporter to attach this to.
        }

        @Composable fun CopyLogsOption() {
            val data = LocalDataStoreCache.current
            val context = LocalContext.current
            NavigationItem(
                title = "Copy logs",
                subtitle = "May contain sensitive data",
                style = NavigationItemStyle.MiscNoArrow,
                navigate = {
                    // ACRA used to build this report; it is written by hand now so
                    // the dialog and the mail sender are not needed.
                    val text = buildString {
                        append("Settings\n")
                        append(Settings.getInstance().current.dump())
                        append("\n\nPreferences\n")
                        data!!.currPreferences.asMap().forEach { (key, value) ->
                            append(key.name).append(" = ").append(value.toString()).append('\n')
                        }
                    }
                    context.copyToClipboard(text)
                },
            )
        }
    }
}
