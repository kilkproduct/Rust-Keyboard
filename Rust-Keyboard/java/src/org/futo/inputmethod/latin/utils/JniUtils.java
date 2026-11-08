/*
 * Copyright (C) 2012 The Android Open Source Project
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package org.futo.inputmethod.latin.utils;

import android.util.Log;

import org.futo.inputmethod.latin.define.JniLibName;

public final class JniUtils {
    private static final String TAG = JniUtils.class.getSimpleName();

    /**
     * Name of the Rust core library. It is optional: when it is missing the
     * keyboard keeps using the C++ engine, so a build without a Rust toolchain
     * still works.
     */
    private static final String RUST_LIB_NAME = "keyboard_core";

    static {
        try {
            System.loadLibrary(JniLibName.JNI_LIB_NAME);
        } catch (UnsatisfiedLinkError ule) {
            Log.e(TAG, "Could not load native library " + JniLibName.JNI_LIB_NAME, ule);
        }
        // Loaded *after* the C++ library on purpose: its JNI_OnLoad registers the
        // dictionary, proximity and traverse-session natives again, replacing the
        // C++ bindings. The C++ library keeps providing the voice-input natives.
        try {
            System.loadLibrary(RUST_LIB_NAME);
            Log.i(TAG, "Rust dictionary engine loaded");
        } catch (UnsatisfiedLinkError ule) {
            Log.i(TAG, "Rust dictionary engine not present, using the C++ engine");
        }
    }

    private JniUtils() {
        // This utility class is not publicly instantiable.
    }

    public static void loadNativeLibrary() {
        // Ensures the static initializer is called
    }
}
