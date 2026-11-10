package org.futo.inputmethod.latin.uix.settings.pages

import android.content.Context
import android.content.Intent
import org.futo.inputmethod.latin.uix.findActivity
import org.futo.inputmethod.latin.uix.settings.IMPORT_RESOURCE_FILE_REQUEST

/**
 * Opens the document picker for a resource this fork can still use: a
 * dictionary or a layout. It used to live in the model-manager package, but
 * that package was only reachable for transformer models, which are gone; the
 * language screen still offers "import resource from file".
 */
fun openModelImporter(context: Context) {
    val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
        addCategory(Intent.CATEGORY_OPENABLE)
        type = "application/octet-stream"
    }

    findActivity(context)!!.startActivityForResult(intent, IMPORT_RESOURCE_FILE_REQUEST)
}
