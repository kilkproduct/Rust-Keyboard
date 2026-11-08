package org.futo.inputmethod.updates

import android.app.job.JobInfo
import android.app.job.JobScheduler
import android.content.ComponentName
import android.content.Context
import android.os.Build
import android.util.Log
import androidx.datastore.preferences.core.edit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
//import okhttp3.OkHttpClient
//import okhttp3.Request
//import okhttp3.internal.closeQuietly
import org.futo.inputmethod.latin.uix.dataStore
import org.futo.inputmethod.latin.uix.getSetting
import org.futo.inputmethod.latin.BuildConfig
import java.lang.Exception

/**
 * This build never contacts an update server. The keyboard has no INTERNET
 * permission and the update check is a deliberate no-op, so update reminders in
 * the settings UI simply never fire. Downloading an update is something the
 * user can still do explicitly from a browser.
 */
suspend fun checkForUpdate(): UpdateResult? {
    return null
}

suspend fun checkForUpdateAndSaveToPreferences(context: Context): Boolean {
    val updateResult = checkForUpdate()
    if(updateResult != null) {
        withContext(Dispatchers.IO) {
            context.dataStore.edit {
                it[LAST_UPDATE_CHECK_RESULT] = Json.encodeToString(updateResult)
                it[LAST_UPDATE_CHECK_FAILED] = false
                it[DEFER_MANUAL_UPDATE_UNTIL] = System.currentTimeMillis() + MANUAL_UPDATE_PERIOD_MS
            }
        }
        return true
    } else {
        context.dataStore.edit {
            it[LAST_UPDATE_CHECK_FAILED] = true

            if(it[DEFER_MANUAL_UPDATE_UNTIL] == null) {
                it[DEFER_MANUAL_UPDATE_UNTIL] = System.currentTimeMillis() + MANUAL_UPDATE_PERIOD_MS
            }
        }

        return false
    }
}

suspend fun retrieveSavedLastUpdateCheckResult(context: Context): UpdateResult? {
    if(!BuildConfig.UPDATE_CHECKING) return null

    return UpdateResult.fromString(context.getSetting(LAST_UPDATE_CHECK_RESULT, ""))
}

const val JOB_ID: Int = 15782788
fun scheduleUpdateCheckingJob(context: Context) {
    if(!BuildConfig.UPDATE_CHECKING_NETWORK) return

    val jobScheduler = context.getSystemService(Context.JOB_SCHEDULER_SERVICE) as JobScheduler

    if(jobScheduler.getPendingJob(JOB_ID) != null) {
        Log.i("UpdateChecking", "Job already scheduled, no need to do anything")
        return
    }

    var jobInfoBuilder = JobInfo.Builder(JOB_ID, ComponentName(context, UpdateCheckingService::class.java))
        .setPeriodic(1000L * 60L * 60L * 12L) // every 12 hours
        //.setRequiredNetworkType(JobInfo.NETWORK_TYPE_UNMETERED) // on unmetered Wi-Fi
        .setPersisted(true) // persist after reboots

    // Update checking has minimum priority
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        jobInfoBuilder = jobInfoBuilder.setPriority(JobInfo.PRIORITY_MIN)
    }

    jobScheduler.schedule(jobInfoBuilder.build())
}