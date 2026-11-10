package org.futo.inputmethod.latin.uix.settings

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.navigation.NavHostController

/**
 * Replacement for the old update dialog.
 *
 * The keyboard no longer checks for updates: it has no network permission at
 * all, so there is nothing to poll and no APK to download in the background.
 * The navigation route is kept so that the existing settings graph still
 * resolves, and the dialog simply says so.
 */
@Composable
fun UpdateDialog(navController: NavHostController) {
    AlertDialog(
        onDismissRequest = { navController.navigateUp() },
        title = { Text("Updates") },
        text = {
            Text(
                "This build does not check for updates and never uses the " +
                    "network. Download a new version in the browser and install " +
                    "it over this one to update."
            )
        },
        confirmButton = {
            TextButton(onClick = { navController.navigateUp() }) {
                Text("Close")
            }
        }
    )
}
