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

    private static String head(ArrayList<String> words, int count) {
        final StringBuilder builder = new StringBuilder();
        for (int index = 0; index < words.size() && index < count; index++) {
            if (builder.length() > 0) {
                builder.append(", ");
            }
            builder.append(words.get(index));
        }
        return builder.toString();
    }

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
        // The native side null terminates, so stop at the first zero: anything
        // past it is left over from a longer word written earlier.
        final StringBuilder builder = new StringBuilder(length);
        for (int index = 0; index < length; index++) {
            final int codePoint = codePoints[offset + index];
            if (codePoint == 0) {
                break;
            }
            builder.append((char) codePoint);
        }
        return builder.toString();
    }


    /** Runs one suggestion query the way LatinIME does and returns the words. */
    private static ArrayList<String> suggest(long dict, long proximity, long session,
            String typed, String[] prevWords, int[] outCountHolder) {
        final int[] input = new int[MAX_WORD_LENGTH];
        java.util.Arrays.fill(input, -1);
        final int inputSize = typed.length();
        for (int index = 0; index < inputSize; index++) {
            input[index] = typed.charAt(index);
        }
        final int[] tapX = new int[Math.max(inputSize, 1)];
        final int[] tapY = new int[Math.max(inputSize, 1)];
        final int[] tapTimes = new int[Math.max(inputSize, 1)];
        final int[] pointerIds = new int[Math.max(inputSize, 1)];
        for (int index = 0; index < inputSize; index++) {
            // Same geometry as letterProximity(): ten keys per row, 100 units
            // apart, so a tap lands on the key it names.
            final int letter = input[index] - 'a';
            if (letter >= 0 && letter < 26) {
                tapX[index] = (letter % 10) * 100 + 50;
                tapY[index] = (letter / 10) * 100 + 50;
            } else {
                tapX[index] = 50;
                tapY[index] = 50;
            }
            tapTimes[index] = 100 + index * 60;
        }
        final int[][] prev = new int[4][];
        final boolean[] prevIsBeginningOfSentence = new boolean[4];
        for (int index = 0; index < prevWords.length && index < 4; index++) {
            prev[index] = codePoints(prevWords[index]);
        }

        final int[] outputCount = new int[1];
        final int[] outputCodePoints = new int[MAX_WORD_LENGTH * MAX_RESULTS];
        final int[] outputScores = new int[MAX_RESULTS];
        final int[] outputIndices = new int[MAX_RESULTS];
        final int[] outputTypes = new int[MAX_RESULTS];
        final int[] outputAutoCommit = new int[1];
        final float[] weight = {1.0f};
        final int[] suggestOptions = {0, 0, 0, 0, 1000};

        BinaryDictionary.getSuggestionsNative(dict, proximity, session, tapX, tapY, tapTimes,
                pointerIds, input, inputSize, suggestOptions, prev, prevIsBeginningOfSentence,
                prevWords.length, outputCount, outputCodePoints, outputScores, outputIndices,
                outputTypes, outputAutoCommit, weight);

        outCountHolder[0] = outputCount[0];
        final ArrayList<String> words = new ArrayList<>();
        for (int index = 0; index < outputCount[0]; index++) {
            int length = 0;
            while (length < MAX_WORD_LENGTH
                    && outputCodePoints[index * MAX_WORD_LENGTH + length] != 0) {
                length++;
            }
            final String word = fromCodePoints(outputCodePoints, index * MAX_WORD_LENGTH, length);
            words.add(word);
            System.out.println("   \"" + typed + "\" -> " + word + " score "
                    + outputScores[index] + " kind " + outputTypes[index]);
        }
        return words;
    }

    /** A proximity map for all lower case letters, laid out like a phone row. */
    private static long letterProximity() {
        final int keyCount = 26;
        final int[] keyCodes = new int[keyCount];
        final int[] keyX = new int[keyCount];
        final int[] keyY = new int[keyCount];
        final int[] keyWidths = new int[keyCount];
        final int[] keyHeights = new int[keyCount];
        final float[] sweetX = new float[keyCount];
        final float[] sweetY = new float[keyCount];
        final float[] sweetRadius = new float[keyCount];
        for (int index = 0; index < keyCount; index++) {
            keyCodes[index] = 'a' + index;
            keyX[index] = (index % 10) * 100;
            keyY[index] = (index / 10) * 100;
            keyWidths[index] = 100;
            keyHeights[index] = 100;
            sweetX[index] = keyX[index] + 50;
            sweetY[index] = keyY[index] + 50;
            sweetRadius[index] = 90.0f;
        }
        return ProximityInfo.create(1000, 300, keyCount, 3, 100, 100, new int[0], keyCount,
                keyX, keyY, keyWidths, keyHeights, keyCodes, sweetX, sweetY, sweetRadius);
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


        // 5b. The same query in the shape the app really uses: LatinIME passes a
        //     MAX_WORD_LENGTH array padded with NOT_A_CODE and a separate
        //     inputSize. Reading the array length instead of inputSize searches
        //     for 48 keys, which is what made the strip show only what was typed.
        final int[] deviceCount = new int[1];
        final ArrayList<String> deviceResults = suggest(dict, proximity, session, "hel",
                new String[0], deviceCount);
        check(deviceCount[0] > 0, "no suggestions when the input array is padded like the app");
        check(!deviceResults.isEmpty() && deviceResults.get(0).startsWith("hel"),
                "unexpected first suggestion with a padded input array: " + deviceResults);
        System.out.println("== padded input array: " + deviceCount[0] + " suggestions");

        // 5c. The real English dictionary, if the caller points at one. This is
        //     the case the settings page and the strip depend on: a word that
        //     should offer an apostrophe variant, and a next word prediction.
        final String englishDictPath = System.getenv("KB_ENGLISH_DICT");
        long englishDict = 0;
        long englishProximity = 0;
        if (englishDictPath != null && new File(englishDictPath).isFile()) {
            englishDict = BinaryDictionary.openNative(englishDictPath, 0,
                    new File(englishDictPath).length(), false);
            check(englishDict != 0, "could not open " + englishDictPath);
            englishProximity = letterProximity();
            check(englishProximity != 0, "could not build the letter proximity");
            DicTraverseSession.initDicTraverseSessionNative(session, englishDict, null, 0);

            final int[] englishHeaderSize = new int[1];
            final int[] englishFormatVersion = new int[1];
            final ArrayList<int[]> englishHeaderKeys = new ArrayList<>();
            final ArrayList<int[]> englishHeaderValues = new ArrayList<>();
            BinaryDictionary.getHeaderInfoNative(englishDict, englishHeaderSize, englishFormatVersion,
                    englishHeaderKeys, englishHeaderValues);
            System.out.println("== english dictionary: format "
                    + BinaryDictionary.getFormatVersionNative(englishDict) + ", header "
                    + englishHeaderSize[0] + " bytes, version " + englishFormatVersion[0] + ", "
                    + englishHeaderKeys.size() + " attributes, file "
                    + new File(englishDictPath).length() + " bytes");
            try (java.io.FileInputStream in = new java.io.FileInputStream(englishDictPath)) {
                final byte[] head = new byte[96];
                final int read = in.read(head);
                final StringBuilder hex = new StringBuilder();
                for (int index = 0; index < read; index++) {
                    hex.append(String.format("%02x", head[index]));
                    if ((index + 1) % 16 == 0) {
                        hex.append('\n');
                    }
                }
                System.out.println("== first bytes:\n" + hex);
                // The trie buffer starts right after the header.
                in.getChannel().position(englishHeaderSize[0]);
                final byte[] trie = new byte[1024];
                final int trieRead = in.read(trie);
                final StringBuilder trieHex = new StringBuilder();
                for (int index = 0; index < trieRead; index++) {
                    trieHex.append(String.format("%02x", trie[index]));
                    if ((index + 1) % 16 == 0) {
                        trieHex.append('\n');
                    }
                }
                System.out.println("== trie start:\n" + trieHex);
            }

            // Does the engine read this dictionary at all? If a lookup fails,
            // the trie parsing is wrong and every suggestion is guesswork.
            for (String probe : new String[] {"its", "it's", "inserts", "the", "italian"}) {
                System.out.println("   probability \"" + probe + "\" = "
                        + BinaryDictionary.getProbabilityNative(englishDict, codePoints(probe)));
            }

            // Walk the whole dictionary once: a wrong node layout shows up here
            // as a nonsense count or as words full of garbage characters.
            int token = 0;
            int words = 0;
            final int[] wordCodePoints = new int[MAX_WORD_LENGTH];
            final boolean[] wordIsBeginningOfSentence = new boolean[1];
            final StringBuilder sample = new StringBuilder();
            // Every word the dictionary contains. A suggestion that is not in
            // here is not a word of this dictionary, which is exactly how the
            // "éf"/"éo" predictions looked: the first letter of the prediction
            // came from a stale buffer, so the rest of the word did not match
            // anything the trie stores.
            final java.util.HashSet<String> dictionaryWords = new java.util.HashSet<>();
            while (words < 400000) {
                token = BinaryDictionary.getNextWordNative(englishDict, token, wordCodePoints,
                        wordIsBeginningOfSentence);
                // A token of zero means "the dictionary ended", exactly like
                // AOSP's getNextWord(); iterating past it restarts from the
                // first word.
                if (token <= 0) {
                    break;
                }
                words++;
                final String walked = fromCodePoints(wordCodePoints, 0, MAX_WORD_LENGTH).trim();
                dictionaryWords.add(walked);
                if (words <= 12) {
                    sample.append(' ').append(walked);
                }
            }
            System.out.println("== dictionary walk: " + words + " words, first:" + sample);
            check(words > 1000, "the dictionary walk stopped after " + words + " words");
            check(token >= 0, "getNextWordNative returned a negative token: " + token);

            final int[] itsCount = new int[1];
            final ArrayList<String> its = suggest(englishDict, englishProximity, session, "its",
                    new String[0], itsCount);
            System.out.println("== \"its\": " + itsCount[0] + " suggestions, apostrophe variant "
                    + (its.contains("it's") ? "present" : "missing"));

            final int[] predictionCount = new int[1];
            final ArrayList<String> prediction = suggest(englishDict, englishProximity, session, "",
                    new String[] {"the"}, predictionCount);
            System.out.println("== after \"the\": " + predictionCount[0] + " predictions: "
                    + head(prediction, 8));

            // Next-word prediction has to depend on the previous word. It used
            // to come from a breadth-first walk of the trie, which is storage
            // order, so every context produced the same "to of in" list.
            final int[] noContextCount = new int[1];
            final ArrayList<String> noContext = suggest(englishDict, englishProximity, session, "",
                    new String[0], noContextCount);
            System.out.println("== no context: " + noContextCount[0] + " predictions: "
                    + head(noContext, 8));

            final int[] afterICount = new int[1];
            final ArrayList<String> afterI = suggest(englishDict, englishProximity, session, "",
                    new String[] {"i"}, afterICount);
            System.out.println("== after \"i\": " + afterICount[0] + " predictions: "
                    + head(afterI, 8));

            check(predictionCount[0] > 0, "no predictions after \"the\"");
            check(noContextCount[0] > 0, "no predictions with no context");
            check(afterICount[0] > 0, "no predictions after \"i\"");
            check(!prediction.equals(noContext),
                    "predictions after \"the\" are identical to the no-context list");
            check(!prediction.equals(afterI),
                    "predictions after \"the\" and after \"i\" are identical");
            int shared = 0;
            for (String word : prediction) {
                if (afterI.contains(word)) {
                    shared++;
                }
            }
            System.out.println("== words suggested for both \"the\" and \"i\": " + shared);

            // 5d. Predictions are built by walking the whole trie, so the prefix
            //     of every candidate has to be carried along explicitly. When it
            //     was read out of one shared buffer instead, every prediction
            //     came back as "the last first letter the walk visited" followed
            //     by the tail of a real word ("éypt", "émail", "é-shirts"): none
            //     of those is a word of the dictionary, so checking membership
            //     catches the whole class of bug.
            int notInDictionary = 0;
            for (String word : prediction) {
                if (!dictionaryWords.contains(word)) {
                    if (notInDictionary < 10) {
                        System.out.println("   prediction that is not a dictionary word: \"" + word + "\"");
                    }
                    notInDictionary++;
                }
            }
            System.out.println("== predictions that are not dictionary words: " + notInDictionary);
            check(notInDictionary == 0, notInDictionary + " predictions are not dictionary words");
            check(!noContext.isEmpty() && dictionaryWords.containsAll(noContext),
                    "the no-context predictions contain words this dictionary does not have");

            final int[] completionCount = new int[1];
            final ArrayList<String> completion = suggest(englishDict, englishProximity, session,
                    "hel", new String[] {"i"}, completionCount);
            System.out.println("== \"hel\" after \"i\": " + completionCount[0] + " suggestions");

            // Every check after all the evidence has been printed.
            check(itsCount[0] > 0, "no suggestions at all for \"its\"");
            check(its.contains("its"), "\"its\" is not suggested for \"its\": " + its);
            check(predictionCount[0] > 0, "no next word predictions after \"the\"");
            check(notInDictionary == 0, notInDictionary
                    + " predictions are not words of the dictionary: " + prediction);
            check(completionCount[0] > 0, "no completions for \"hel\": " + completion);
        } else {
            System.out.println("== no KB_ENGLISH_DICT, skipping the real dictionary checks");
        }

        // 6. Cleaning up must not corrupt the heap either.
        DicTraverseSession.releaseDicTraverseSessionNative(session);
        ProximityInfo.release(proximity);
        BinaryDictionary.closeNative(dict);
        if (englishDict != 0) {
            BinaryDictionary.closeNative(englishDict);
        }
        if (englishProximity != 0) {
            ProximityInfo.release(englishProximity);
        }

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
