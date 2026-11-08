// Test double for java/src/org/futo/inputmethod/latin/utils/BinaryDictionaryUtils.java.

package org.futo.inputmethod.latin.utils;

public final class BinaryDictionaryUtils {

    private BinaryDictionaryUtils() {}

    private static native boolean createEmptyDictFileNative(String filePath, long dictVersion,
            String locale, String[] attributeKeyStringArray, String[] attributeValueStringArray);

    static native float calcNormalizedScoreNative(int[] before, int[] after, int score);
    static native int setCurrentTimeForTestNative(int currentTime);

    /** Mirrors the real helper, minus the Android-only DictionaryHeader round trip. */
    public static boolean createEmptyDictFile(String filePath, long dictVersion, String locale) {
        final String[] keys = {"dictionary", "version", "locale"};
        final String[] values = {"user", Long.toString(dictVersion), locale};
        return createEmptyDictFileNative(filePath, dictVersion, locale, keys, values);
    }
}
