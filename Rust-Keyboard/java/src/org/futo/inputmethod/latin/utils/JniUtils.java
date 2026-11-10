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

public final class JniUtils {
    private static final String TAG = JniUtils.class.getSimpleName();

    /**
     * The only native library of this build. The C++ engine (`jni_latinime`)
     * it used to sit next to is gone, so there is nothing to try after this.
     */
    private static final String RUST_LIB_NAME = "keyboard_core";

    static {
        try {
            System.loadLibrary(RUST_LIB_NAME);
            Log.i(TAG, "Rust dictionary engine loaded");
        } catch (UnsatisfiedLinkError ule) {
            // Do not swallow this: the build is supposed to refuse to package an
            // APK without this library, so reaching here means the installation
            // is damaged. Every native call will fail the same way, and this is
            // the message that says why.
            Log.e(TAG, "Could not load native library " + RUST_LIB_NAME, ule);
        }
    }

    private JniUtils() {
        // This utility class is not publicly instantiable.
    }

    public static void loadNativeLibrary() {
        // Ensures the static initializer is called
    }
}
