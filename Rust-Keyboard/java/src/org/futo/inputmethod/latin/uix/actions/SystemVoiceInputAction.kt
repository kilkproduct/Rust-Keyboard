package org.futo.inputmethod.latin.uix.actions

import org.futo.inputmethod.latin.R
import org.futo.inputmethod.latin.uix.Action

/**
 * Dictation through the system speech recogniser.
 *
 * The bundled whisper engine (and its 79 MB of models) is not part of this
 * build, so this is the only voice-input action.
 */
val SystemVoiceInputAction = Action(
    icon = R.drawable.mic_fill,
    name = R.string.action_system_voice_input_title,
    simplePressImpl = { it, _ ->
        it.triggerSystemVoiceInput()
    },
    persistentState = null,
    windowImpl = null,
    shownInEditor = false
)
