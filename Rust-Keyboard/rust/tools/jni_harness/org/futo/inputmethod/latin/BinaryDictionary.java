// Test double for the real org.futo.inputmethod.latin.BinaryDictionary.
//
// The JNI layer registers its functions against this exact class name and these
// exact signatures, so a desktop JVM can exercise the engine without Android.
// Keep the signatures in sync with java/src/org/futo/inputmethod/latin/BinaryDictionary.java.

package org.futo.inputmethod.latin;

import java.util.ArrayList;

import org.futo.inputmethod.latin.utils.WordInputEventForPersonalization;

public final class BinaryDictionary {

    private BinaryDictionary() {}

    static native long openNative(String sourceDir, long dictOffset, long dictSize,
            boolean isUpdatable);

    static native long createOnMemoryNative(long formatVersion, String locale,
            String[] attributeKeyStringArray, String[] attributeValueStringArray);

    static native void getHeaderInfoNative(long dict, int[] outHeaderSize, int[] outFormatVersion,
            ArrayList<int[]> outAttributeKeys, ArrayList<int[]> outAttributeValues);

    static native boolean flushNative(long dict, String filePath);
    static native void closeNative(long dict);
    static native int getFormatVersionNative(long dict);
    static native int getProbabilityNative(long dict, int[] word);
    static native int getMaxProbabilityOfExactMatchesNative(long dict, int[] word);

    static native void getSuggestionsNative(long dict, long proximityInfo, long traverseSession,
            int[] xCoordinates, int[] yCoordinates, int[] times, int[] pointerIds,
            int[] inputCodePoints, int inputSize, int[] suggestOptions,
            int[][] prevWordCodePointArrays, boolean[] isBeginningOfSentenceArray,
            int prevWordCount, int[] outputSuggestionCount, int[] outputCodePoints,
            int[] outputScores, int[] outputIndices, int[] outputTypes,
            int[] outputAutoCommitFirstWordConfidence,
            float[] inOutWeightOfLangModelVsSpatialModel);

    static native boolean addUnigramEntryNative(long dict, int[] word, int probability,
            int[] shortcutTarget, int shortcutProbability, boolean isBeginningOfSentence,
            boolean isNotAWord, boolean isPossiblyOffensive, int timestamp);

    static native boolean removeUnigramEntryNative(long dict, int[] word);
    static native String getPropertyNative(long dict, String query);
    static native boolean isCorruptedNative(long dict);

    static native int updateEntriesForInputEventsNative(long dict,
            WordInputEventForPersonalization[] inputEvents, int startIndex);
}
