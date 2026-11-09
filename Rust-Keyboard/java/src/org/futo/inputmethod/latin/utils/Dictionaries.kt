package org.futo.inputmethod.latin.utils

import android.content.Context
import android.content.res.AssetFileDescriptor
import android.content.res.Resources.NotFoundException
import android.util.Log
import org.futo.inputmethod.latin.AssetFileAddress
import org.futo.inputmethod.latin.BundleHelper
import org.futo.inputmethod.latin.R
import java.io.BufferedInputStream
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.util.Locale
import java.util.zip.GZIPInputStream

object Dictionaries {
    enum class DictionaryKind(val candidateNameGenerator: (Locale) -> List<String>) {
        BinaryDictionary({ listOf(
            "main_" + it.toString().lowercase(),
            "main_" + it.language.lowercase())
        }),

        Mozc({
            if(it.language == "ja") listOf("builtin_mozc_data") else emptyList()
        }),

        Any({ locale ->
            DictionaryKind.entries.filter { it != Any }.flatMap { it.candidateNameGenerator(locale) }
        })
    }

    // The English dictionary is shipped gzipped in the assets and unpacked into
    // the app's own files directory on first use: keeping it mmap-able inside
    // the package meant aapt stored a megabyte uncompressed. Bump the version
    // when the bundled dictionary changes so installed copies are refreshed.
    // aapt2 unpacks assets whose name ends in .gz and drops the suffix, so the
    // archive is called .gzip to survive packaging. The loader below accepts any
    // of the names and either encoding, because packaging details like that are
    // not worth a broken dictionary.
    private const val BUNDLED_DICT_NAME = "main_en.dict"
    private val BUNDLED_DICT_ASSETS = listOf(
            "dict/main_en.dict.gzip",
            "dict/main_en.dict.gz",
            "dict/main_en.dict")
    private const val BUNDLED_DICT_VERSION = "2"

    private fun openBundledDictionaryAsset(context: Context): InputStream? {
        for (name in BUNDLED_DICT_ASSETS) {
            try {
                val stream = BufferedInputStream(context.assets.open(name))
                stream.mark(2)
                val first = stream.read()
                val second = stream.read()
                stream.reset()
                return if (first == 0x1f && second == 0x8b) GZIPInputStream(stream) else stream
            } catch (e: IOException) {
                // try the next name
            }
        }
        Log.e("Dictionaries", "the bundled dictionary asset is missing")
        return null
    }

    @Volatile
    private var extractedDictionary: File? = null

    private fun getBundledDictionaryFile(context: Context): File? {
        extractedDictionary?.let { if (it.isFile) return it }

        val dir = File(context.filesDir, "bundled-dict")
        val target = File(dir, BUNDLED_DICT_NAME)
        val stamp = File(dir, "$BUNDLED_DICT_NAME.version")

        val upToDate = target.isFile && target.length() > 0L && stamp.isFile &&
                (stamp.readText().trim() == BUNDLED_DICT_VERSION)
        if (!upToDate) {
            try {
                dir.mkdirs()
                val temporary = File(dir, "$BUNDLED_DICT_NAME.tmp")
                val source = openBundledDictionaryAsset(context) ?: return null
                source.use { asset ->
                    temporary.outputStream().use { sink -> asset.copyTo(sink) }
                }
                // A truncated or empty unpack must not be handed to the engine:
                // it would silently suggest nothing at all.
                if (temporary.length() < 100_000L) {
                    Log.e("Dictionaries", "unpacked dictionary is too small: ${temporary.length()} bytes")
                    temporary.delete()
                    return null
                }
                if (target.isFile) target.delete()
                if (!temporary.renameTo(target)) {
                    temporary.delete()
                    return null
                }
                stamp.writeText(BUNDLED_DICT_VERSION)
                Log.i("Dictionaries", "unpacked the bundled dictionary: ${target.length()} bytes")
            } catch (e: IOException) {
                Log.e("Dictionaries", "could not unpack the bundled dictionary", e)
                return null
            }
        }

        extractedDictionary = target
        return target
    }

    fun getDictionaryIfExists(context: Context, locale: Locale?, kind: DictionaryKind): AssetFileAddress? {
        if(locale == null) return null

        if(kind == DictionaryKind.BinaryDictionary && locale.language == Locale.ENGLISH.language) {
            getBundledDictionaryFile(context)?.let { return AssetFileAddress.makeFromFileName(it.absolutePath) }
        }

        return kind.candidateNameGenerator(locale).firstNotNullOfOrNull {
            BundleHelper.obtainSplitAssetFileDescriptor(context, it, locale)
        }
    }

    fun getFallbackDictionary(context: Context): AssetFileAddress? {
        getBundledDictionaryFile(context)?.let { return AssetFileAddress.makeFromFileName(it.absolutePath) }

        var afd: AssetFileDescriptor? = null
        try {
            val resId: Int = R.raw.main
            if (0 == resId) return null
            afd = context.resources.openRawResourceFd(resId)
            if (afd == null) {
                Log.e("Dictionaries", "Found the resource but it is compressed. resId=" + resId)
                return null
            }
            val sourceDir = context.getApplicationInfo().sourceDir

            val packagePath = File(sourceDir)
            if (!packagePath.isFile()) {
                Log.e("Dictionaries", "sourceDir is not a file: " + sourceDir)
                return null
            }

            return AssetFileAddress(sourceDir, afd.startOffset, afd.length)
        } catch (e: NotFoundException) {
            Log.e("Dictionaries", "Could not find the resource")
            return null
        } finally {
            if (afd != null) {
                try {
                    afd.close()
                } catch (e: IOException) {
                    /* IOException on close ? What am I supposed to do ? */
                }
            }
        }
    }
}