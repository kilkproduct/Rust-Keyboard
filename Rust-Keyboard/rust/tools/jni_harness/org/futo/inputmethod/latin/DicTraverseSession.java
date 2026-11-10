// Test double for the real java/src/org/futo/inputmethod/latin/DicTraverseSession.java.

package org.futo.inputmethod.latin;

public final class DicTraverseSession {

    private DicTraverseSession() {}

    static native long setDicTraverseSessionNative(String locale, long dictSize);

    static native void initDicTraverseSessionNative(long nativeDicTraverseSession, long dictionary,
            int[] previousWord, int previousWordLength);

    static native void releaseDicTraverseSessionNative(long nativeDicTraverseSession);
}
