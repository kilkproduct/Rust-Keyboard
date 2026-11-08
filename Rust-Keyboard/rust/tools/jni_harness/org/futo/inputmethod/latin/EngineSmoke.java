// Desktop smoke test for the Rust keyboard engine's JNI surface.
//
// The APK cannot be exercised on a build machine, but the JNI layer can: the
// same class names, the same native signatures and the same call order the
// Android code uses are replayed here against the host build of
// libkeyboard_core.so. This is what catches ABI mistakes (wrong JNI slot,
// variadic call used where the `A` variant is required) and memory errors before
// they reach a phone.

package org.futo.inputmethod.latin;

import java.io.File;
import java.util.ArrayList;

import org.futo.inputmethod.keyboard.ProximityInfo;
import org.futo.inputmethod.latin.utils.BinaryDictionaryUtils;

public final class EngineSmoke {

    private static final int MAX_WORD_LENGTH = 48;
    private static final int MAX_RESULTS = 40;

    private EngineSmoke() {}

    private static void check(boolean condition, String message) {
        if (!condition) {
            throw new AssertionError(message);
        }
    }

    private static int[] codePoints(String word) {
        final int[] out = new int[word.length()];
        for (int index = 0; index < word.length(); index++) {
            out[index] = word.charAt(index);
        }
        return out;
    }

    private static String fromCodePoints(int[] codePoints, int offset, int length) {
        final StringBuilder builder = new StringBuilder(length);
        for (int index = 0; index < length; index++) {
            builder.append((char) codePoints[offset + index]);
        }
        return builder.toString();
    }

    public static void main(String[] args) throws Exception {
        System.loadLibrary("keyboard_core");
        System.out.println("== libkeyboard_core loaded");

        final File directory = new File(System.getProperty("java.io.tmpdir"),
                "kb-jni-smoke-" + System.nanoTime());
        check(directory.mkdirs(), "cannot create " + directory);
        final String dictPath = new File(directory, "user.dict").getAbsolutePath();

        // 1. createEmptyDictFileNative has to produce a real dictionary file.
        check(BinaryDictionaryUtils.createEmptyDictFile(dictPath, 403, "en"),
                "createEmptyDictFile returned false");
        check(new File(dictPath).length() > 12, "the dictionary file has no header");
        System.out.println("== empty dictionary created: " + new File(dictPath).length() + " bytes");

        // 2. Opening it and reading the header is the call that used to abort the
        //    process: getHeaderInfoNative appends to two ArrayLists through JNI.
        long dict = BinaryDictionary.openNative(dictPath, 0, new File(dictPath).length(), true);
        check(dict != 0, "openNative returned a null handle");

        final int[] headerSize = new int[1];
        final int[] formatVersion = new int[1];
        final ArrayList<int[]> attributeKeys = new ArrayList<>();
        final ArrayList<int[]> attributeValues = new ArrayList<>();
        BinaryDictionary.getHeaderInfoNative(dict, headerSize, formatVersion, attributeKeys,
                attributeValues);
        check(headerSize[0] > 0, "header size was not reported");
        check(formatVersion[0] == 403, "unexpected format version " + formatVersion[0]);
        check(attributeKeys.size() > 0, "no header attributes were returned");
        check(attributeKeys.size() == attributeValues.size(),
                "attribute key and value lists differ in length");
        System.out.println("== header read: version " + formatVersion[0] + ", "
                + attributeKeys.size() + " attributes, size " + headerSize[0]);

        // 3. Personalising the dictionary.
        check(BinaryDictionary.addUnigramEntryNative(dict, codePoints("hello"), 200, null, 0,
                false, false, false, 0), "adding \"hello\" failed");
        check(BinaryDictionary.addUnigramEntryNative(dict, codePoints("help"), 180, null, 0,
                false, false, false, 0), "adding \"help\" failed");
        check(BinaryDictionary.getProbabilityNative(dict, codePoints("hello")) > 0,
                "\"hello\" has no probability after being added");
        check(BinaryDictionary.getMaxProbabilityOfExactMatchesNative(dict, codePoints("hello")) > 0,
                "exact match probability lookup failed");
        System.out.println("== unigrams added and readable");

        // 4. Flushing writes the sidecar log; reopening must see the words again.
        check(BinaryDictionary.flushNative(dict, dictPath), "flushNative returned false");
        BinaryDictionary.closeNative(dict);

        dict = BinaryDictionary.openNative(dictPath, 0, new File(dictPath).length(), true);
        check(dict != 0, "reopening the dictionary failed");
        check(BinaryDictionary.getProbabilityNative(dict, codePoints("hello")) > 0,
                "personalised word was lost when the dictionary was reopened");
        check(!BinaryDictionary.isCorruptedNative(dict), "the dictionary reports itself corrupt");
        System.out.println("== personalisation survived a reopen");

        // 5. Suggestions, exactly as LatinIME asks for them: a three key tap on
        //    "hel" over a five key layout.
        final int keyCount = 5;
        final int[] keyCodes = {'h', 'e', 'l', 'o', 'p'};
        final int[] keyX = new int[keyCount];
        final int[] keyY = new int[keyCount];
        final int[] keyWidths = new int[keyCount];
        final int[] keyHeights = new int[keyCount];
        final float[] sweetX = new float[keyCount];
        final float[] sweetY = new float[keyCount];
        final float[] sweetRadius = new float[keyCount];
        for (int index = 0; index < keyCount; index++) {
            keyX[index] = index * 100;
            keyWidths[index] = 100;
            keyHeights[index] = 100;
            sweetX[index] = index * 100 + 50;
            sweetY[index] = 50.0f;
            sweetRadius[index] = 50.0f;
        }
        final long proximity = ProximityInfo.create(500, 100, keyCount, 1, 100, 100,
                new int[0], keyCount, keyX, keyY, keyWidths, keyHeights, keyCodes, sweetX, sweetY,
                sweetRadius);
        check(proximity != 0, "setProximityInfoNative returned a null handle");

        final long session = DicTraverseSession.setDicTraverseSessionNative("en", 4096);
        check(session != 0, "setDicTraverseSessionNative returned a null handle");
        DicTraverseSession.initDicTraverseSessionNative(session, dict, null, 0);

        final int[] taps = {'h', 'e', 'l'};
        final int inputSize = taps.length;
        final int[] tapX = new int[inputSize];
        final int[] tapY = new int[inputSize];
        final int[] tapTimes = new int[inputSize];
        final int[] pointerIds = new int[inputSize];
        for (int index = 0; index < inputSize; index++) {
            tapX[index] = (int) sweetX[index];
            tapY[index] = (int) sweetY[index];
            tapTimes[index] = 100 + index * 60;
        }

        final int[] outputCount = new int[1];
        final int[] outputCodePoints = new int[MAX_WORD_LENGTH * MAX_RESULTS];
        final int[] outputScores = new int[MAX_RESULTS];
        final int[] outputIndices = new int[MAX_RESULTS];
        final int[] outputTypes = new int[MAX_RESULTS];
        final int[] outputAutoCommit = new int[1];
        final float[] weight = {1.0f};
        final int[] suggestOptions = {0, 0, 0, 0, 1000};
        final int[][] prevWords = new int[4][];
        final boolean[] prevWordIsBeginningOfSentence = new boolean[4];

        BinaryDictionary.getSuggestionsNative(dict, proximity, session, tapX, tapY, tapTimes,
                pointerIds, taps, inputSize, suggestOptions, prevWords,
                prevWordIsBeginningOfSentence, 0, outputCount, outputCodePoints, outputScores,
                outputIndices, outputTypes, outputAutoCommit, weight);

        check(outputCount[0] > 0, "no suggestions for \"hel\" even though \"hello\" is known");
        final String first = fromCodePoints(outputCodePoints, 0, MAX_WORD_LENGTH);
        check(first.startsWith("hel"), "unexpected first suggestion: " + first);
        for (int index = 0; index < outputCount[0]; index++) {
            final String word = fromCodePoints(outputCodePoints, index * MAX_WORD_LENGTH,
                    MAX_WORD_LENGTH);
            System.out.println("   suggestion " + index + ": " + word.trim() + " score "
                    + outputScores[index] + " kind " + outputTypes[index]);
        }
        System.out.println("== " + outputCount[0] + " suggestions returned, first is \""
                + first.trim() + "\"");

        // 6. Cleaning up must not corrupt the heap either.
        DicTraverseSession.releaseDicTraverseSessionNative(session);
        ProximityInfo.release(proximity);
        BinaryDictionary.closeNative(dict);

        final File[] leftovers = directory.listFiles();
        if (leftovers != null) {
            for (File file : leftovers) {
                //noinspection ResultOfMethodCallIgnored
                file.delete();
            }
        }
        //noinspection ResultOfMethodCallIgnored
        directory.delete();

        System.out.println("== JNI smoke test passed");
    }
}
