// Test double for java/src/org/futo/inputmethod/latin/utils/WordInputEventForPersonalization.java.
// Only needed so updateEntriesForInputEventsNative has its parameter type, which is
// what the registered JNI signature refers to.

package org.futo.inputmethod.latin.utils;

public final class WordInputEventForPersonalization {

    public final int[] mTargetWord;
    public final int[] mSurroundingWords;
    public final int mEventType;
    public final int mTimestamp;

    public WordInputEventForPersonalization(int[] targetWord, int[] surroundingWords,
            int eventType, int timestamp) {
        mTargetWord = targetWord;
        mSurroundingWords = surroundingWords;
        mEventType = eventType;
        mTimestamp = timestamp;
    }
}
