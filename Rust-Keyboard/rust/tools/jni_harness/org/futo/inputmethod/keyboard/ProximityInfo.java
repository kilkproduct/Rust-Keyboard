// Test double for the real java/src/org/futo/inputmethod/keyboard/ProximityInfo.java.

package org.futo.inputmethod.keyboard;

public final class ProximityInfo {

    private ProximityInfo() {}

    private static native long setProximityInfoNative(int displayWidth, int displayHeight,
            int gridWidth, int gridHeight, int mostCommonKeyWidth, int mostCommonKeyHeight,
            int[] proximityCharsArray, int keyCount, int[] keyXCoordinates, int[] keyYCoordinates,
            int[] keyWidths, int[] keyHeights, int[] keyCharCodes, float[] sweetSpotCenterXs,
            float[] sweetSpotCenterYs, float[] sweetSpotRadii);

    private static native void releaseProximityInfoNative(long nativeProximityInfo);

    /** Public helper so the smoke test can build a ProximityInfo without Android. */
    public static long create(int displayWidth, int displayHeight, int gridWidth, int gridHeight,
            int mostCommonKeyWidth, int mostCommonKeyHeight, int[] proximityChars, int keyCount,
            int[] keyX, int[] keyY, int[] keyWidths, int[] keyHeights, int[] keyCharCodes,
            float[] sweetSpotXs, float[] sweetSpotYs, float[] sweetSpotRadii) {
        return setProximityInfoNative(displayWidth, displayHeight, gridWidth, gridHeight,
                mostCommonKeyWidth, mostCommonKeyHeight, proximityChars, keyCount, keyX, keyY,
                keyWidths, keyHeights, keyCharCodes, sweetSpotXs, sweetSpotYs, sweetSpotRadii);
    }

    public static void release(long handle) {
        releaseProximityInfoNative(handle);
    }
}
