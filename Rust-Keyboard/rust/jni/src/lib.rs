//! JNI bridge between the Kotlin/Java keyboard and [`kb_core`].
//!
//! The library is loaded *after* `libjni_latinime.so` (see
//! `JniUtils.loadNativeLibrary()`), and registers the dictionary, proximity and
//! traverse-session natives again. `RegisterNatives` is documented to replace a
//! previous binding for the same method, so the Rust implementation takes over
//! from the C++ engine while the C++ library keeps providing the voice-input
//! natives. If this library is missing, the C++ engine keeps serving every call
//! -- the fallback is automatic.
//!
//! Everything here follows the object-handle convention of the original code:
//! a `long` returned to Java is a `Box`-ed Rust value, and every entry point is
//! written to be panic-free (a panic across the FFI boundary aborts the app).

mod env;
mod jni_sys;

use core::ffi::c_void;
use std::cell::RefCell;

use env::{
    JBoolean, JBooleanArray, JClass, JFloatArray, JInt, JIntArray, JLong, JObject, JObjectArray,
    JString, JniEnvPtr, JavaVmPtr,
};
use kb_core::dict::Dictionary;
use kb_core::proximity::ProximityInfo;
use kb_core::suggest::{self, SuggestOptions, SuggestedWord};
use kb_core::MAX_RESULTS;

#[cfg(target_os = "android")]
extern "C" {
    fn __android_log_print(
        priority: i32,
        tag: *const core::ffi::c_char,
        format: *const core::ffi::c_char,
        ...
    ) -> i32;
}

const LOG_ERROR: i32 = 6;
const LOG_INFO: i32 = 4;

/// Writes one line to logcat on Android, to stderr everywhere else (so that a
/// host `cargo build` of the crate links without the Android liblog).
fn platform_log(priority: i32, message: &str) {
    #[cfg(target_os = "android")]
    unsafe {
        let tag = std::ffi::CString::new("KbRust").unwrap_or_else(|_| std::ffi::CString::new("KbRust").unwrap());
        let format = std::ffi::CString::new("%s").unwrap_or_else(|_| std::ffi::CString::new("%s").unwrap());
        let text = std::ffi::CString::new(message)
            .unwrap_or_else(|_| std::ffi::CString::new("unprintable log message").unwrap());
        __android_log_print(priority, tag.as_ptr(), format.as_ptr(), text.as_ptr());
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = priority;
        eprintln!("[kb-rust] {}", message);
    }
}

macro_rules! log_error {
    ($($arg:tt)*) => {{
        platform_log(LOG_ERROR, &format!($($arg)*));
    }};
}

macro_rules! log_info {
    ($($arg:tt)*) => {{
        platform_log(LOG_INFO, &format!($($arg)*));
    }};
}

/// A boxed dictionary; the pointer is what Java stores in its `long`.
struct NativeDictionary {
    dict: Dictionary,
}

fn handle_of(dict: Dictionary) -> JLong {
    Box::into_raw(Box::new(NativeDictionary { dict })) as JLong
}

/// # Safety
/// `handle` must come from [`handle_of`] and must not have been closed.
unsafe fn dict_ref<'a>(handle: JLong) -> Option<&'a NativeDictionary> {
    if handle == 0 {
        return None;
    }
    let pointer = handle as *const NativeDictionary;
    Some(&*pointer)
}

/// # Safety
/// As [`dict_ref`], but hands out a mutable pointer.
unsafe fn dict_mut<'a>(handle: JLong) -> Option<&'a mut NativeDictionary> {
    if handle == 0 {
        return None;
    }
    let pointer = handle as *mut NativeDictionary;
    Some(&mut *pointer)
}

// ---------------------------------------------------------------------------
// Scratch buffers
//
// The keyboard calls `getSuggestionsNative` on every keystroke. Keeping the
// working buffers in thread-local storage means steady-state typing performs
// no heap allocation at all.
// ---------------------------------------------------------------------------

struct Scratch {
    points: Vec<(i32, i32, i32, i32)>,
    code_points: Vec<i32>,
    prev_words: Vec<Vec<i32>>,
    results: Vec<SuggestedWord>,
    /// Window of the dictionary word walk used by `getNextWordNative`.
    /// Only a small window is held, never the whole dictionary: an English
    /// dictionary would otherwise cost several megabytes per process.
    next_word_cache: Vec<(Vec<i32>, i32)>,
    /// Token of the first word in `next_word_cache`.
    next_word_start: usize,
}

/// How many words of the walk `getNextWordNative` keeps resident at a time.
const NEXT_WORD_WINDOW: usize = 1024;

impl Scratch {
    fn new() -> Scratch {
        Scratch {
            points: Vec::with_capacity(64),
            code_points: Vec::with_capacity(64),
            prev_words: Vec::with_capacity(4),
            results: Vec::with_capacity(MAX_RESULTS),
            next_word_cache: Vec::with_capacity(NEXT_WORD_WINDOW),
            next_word_start: 0,
        }
    }
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::new());
}

// ---------------------------------------------------------------------------

fn read_int_array(env: JniEnvPtr, array: JIntArray) -> Vec<i32> {
    let length = env::array_length(env, array);
    let mut buffer = vec![0i32; length];
    if !env::get_int_array_region(env, array, 0, &mut buffer) {
        buffer.clear();
    }
    buffer
}

fn read_int_array_into(env: JniEnvPtr, array: JIntArray, out: &mut [i32]) -> bool {
    if out.is_empty() {
        return true;
    }
    env::get_int_array_region(env, array, 0, out)
}

fn read_object_int_array(env: JniEnvPtr, array: JObjectArray, index: usize) -> Vec<i32> {
    let element = env::object_array_element(env, array, index);
    if element.is_null() {
        return Vec::new();
    }
    let values = read_int_array(env, element as JIntArray);
    env::delete_local_ref(env, element);
    values
}

fn read_prev_words(env: JniEnvPtr, prev_words: JObjectArray, count: usize) -> Vec<Vec<i32>> {
    let mut words = Vec::with_capacity(count.min(4));
    for index in 0..count.min(4) {
        words.push(read_object_int_array(env, prev_words, index));
    }
    words
}

/// Mirrors `outputCodePoints(...)` from `jni_common.cpp`: copy at most
/// `max_length` code points and null terminate only when they fit, so the very
/// last slot never writes past the end of the Java array.
fn write_code_points(
    env: JniEnvPtr,
    array: JIntArray,
    start: usize,
    max_length: usize,
    code_points: &[i32],
) {
    let length = code_points.len().min(max_length);
    if length > 0 {
        env::set_int_array_region(env, array, start, &code_points[..length]);
    }
    if length < max_length {
        let terminator = [0i32];
        env::set_int_array_region(env, array, start + length, &terminator);
    }
}

// ---------------------------------------------------------------------------
// BinaryDictionary
// ---------------------------------------------------------------------------

unsafe extern "system" fn open_native(
    env: JniEnvPtr,
    _class: JClass,
    path: JString,
    offset: JLong,
    size: JLong,
    is_updatable: JBoolean,
) -> JLong {
    let path = match env::get_string(env, path) {
        Some(value) => value,
        None => return 0,
    };
    if size < 0 || offset < 0 {
        return 0;
    }
    match Dictionary::open(&path, offset as u64, size as usize, is_updatable != 0) {
        Some(dict) => {
            if dict.corrupted {
                log_error!("dictionary {} is corrupted or unsupported", path);
            }
            handle_of(dict)
        }
        None => {
            log_error!("cannot open dictionary {}", path);
            0
        }
    }
}

unsafe extern "system" fn create_on_memory_native(
    _env: JniEnvPtr,
    _class: JClass,
    format_version: JLong,
    _locale: JString,
    _attribute_keys: JObjectArray,
    _attribute_values: JObjectArray,
) -> JLong {
    let _ = format_version;
    // An "on memory" dictionary is nothing but the personalisation log: the
    // trie it would have had is replaced by `kb_core::user`.
    match Dictionary::new_in_memory(true) {
        Some(dict) => handle_of(dict),
        None => 0,
    }
}

unsafe extern "system" fn get_header_info_native(
    env: JniEnvPtr,
    _class: JClass,
    handle: JLong,
    out_header_size: JIntArray,
    out_format_version: JIntArray,
    out_attribute_keys: JObject,
    out_attribute_values: JObject,
) {
    let dictionary = match dict_ref(handle) {
        Some(value) => value,
        None => return,
    };
    env::set_int_array_region(env, out_header_size, 0, &[dictionary.dict.header_size() as i32]);
    env::set_int_array_region(
        env,
        out_format_version,
        0,
        &[dictionary.dict.format_version()],
    );

    // The Java side passes two `ArrayList<int[]>`; fill them in place with
    // null terminated code point arrays, exactly like the C++ engine does.
    let keys_add = match env::add_method_of(env, out_attribute_keys) {
        Some(value) => value,
        None => return,
    };
    let values_add = match env::add_method_of(env, out_attribute_values) {
        Some(value) => value,
        None => return,
    };

    for (key, value) in dictionary.dict.header().attributes.iter() {
        let mut key_code_points: Vec<i32> = key.chars().map(|c| c as i32).collect();
        key_code_points.push(0);
        let mut value_code_points = value.clone();
        value_code_points.push(0);
        let key_array = env::new_int_array(env, &key_code_points);
        let value_array = env::new_int_array(env, &value_code_points);
        env::add_object_to_list(env, out_attribute_keys, keys_add, key_array);
        env::add_object_to_list(env, out_attribute_values, values_add, value_array);
        env::delete_local_ref(env, key_array);
        env::delete_local_ref(env, value_array);
    }
}

unsafe extern "system" fn flush_native(
    env: JniEnvPtr,
    _class: JClass,
    handle: JLong,
    path: JString,
) -> JBoolean {
    let dictionary = match dict_mut(handle) {
        Some(value) => value,
        None => return 0,
    };
    let path = env::get_string(env, path).unwrap_or_default();
    match dictionary.dict.user.as_mut() {
        Some(user) => {
            user.set_path(&path);
            if user.flush() {
                1
            } else {
                0
            }
        }
        None => 0,
    }
}

unsafe extern "system" fn needs_to_run_gc_native(
    _env: JniEnvPtr,
    _class: JClass,
    handle: JLong,
    _minds_block_by_gc: JBoolean,
) -> JBoolean {
    match dict_ref(handle) {
        Some(dictionary) => match dictionary.dict.user.as_ref() {
            Some(user) => {
                if user.needs_gc() {
                    1
                } else {
                    0
                }
            }
            None => 0,
        },
        None => 0,
    }
}

unsafe extern "system" fn flush_with_gc_native(
    env: JniEnvPtr,
    class: JClass,
    handle: JLong,
    path: JString,
) -> JBoolean {
    // The log is self-compacting (records are updated in place, the oldest is
    // evicted when full), so "with GC" is the same operation.
    flush_native(env, class, handle, path)
}

unsafe extern "system" fn close_native(_env: JniEnvPtr, _class: JClass, handle: JLong) {
    if handle == 0 {
        return;
    }
    let pointer = handle as *mut NativeDictionary;
    drop(Box::from_raw(pointer));
}

unsafe extern "system" fn get_format_version_native(
    _env: JniEnvPtr,
    _class: JClass,
    handle: JLong,
) -> JInt {
    match dict_ref(handle) {
        Some(dictionary) => dictionary.dict.format_version(),
        None => 0,
    }
}

unsafe extern "system" fn get_probability_native(
    env: JniEnvPtr,
    _class: JClass,
    handle: JLong,
    word: JIntArray,
) -> JInt {
    let dictionary = match dict_ref(handle) {
        Some(value) => value,
        None => return kb_core::NOT_A_PROBABILITY,
    };
    let code_points = read_int_array(env, word);
    if code_points.is_empty() {
        return kb_core::NOT_A_PROBABILITY;
    }
    if let Some(user_dict) = dictionary.dict.user.as_ref() {
        if let Some(probability) = user_dict.lookup_any_context(&code_points) {
            return probability;
        }
    }
    dictionary.dict.get_probability(&code_points)
}

unsafe extern "system" fn get_max_probability_of_exact_matches_native(
    env: JniEnvPtr,
    _class: JClass,
    handle: JLong,
    word: JIntArray,
) -> JInt {
    let dictionary = match dict_ref(handle) {
        Some(value) => value,
        None => return kb_core::NOT_A_PROBABILITY,
    };
    let code_points = read_int_array(env, word);
    let mut best = dictionary.dict.get_max_probability_of_exact_matches(&code_points);
    // Words the user taught us live in the personalisation log, not in the trie,
    // so they have to be considered here as well; getProbabilityNative does.
    if let Some(user_dict) = dictionary.dict.user.as_ref() {
        if let Some(probability) = user_dict.lookup_any_context(&code_points) {
            if probability > best {
                best = probability;
            }
        }
    }
    best
}

unsafe extern "system" fn get_ngram_probability_native(
    env: JniEnvPtr,
    _class: JClass,
    handle: JLong,
    prev_words: JObjectArray,
    _is_beginning_of_sentence: JBooleanArray,
    word: JIntArray,
) -> JInt {
    let dictionary = match dict_ref(handle) {
        Some(value) => value,
        None => return kb_core::NOT_A_PROBABILITY,
    };
    let count = env::array_length(env, prev_words);
    let previous = read_prev_words(env, prev_words, count);
    let code_points = read_int_array(env, word);
    if code_points.is_empty() {
        return kb_core::NOT_A_PROBABILITY;
    }
    let (terminal_id, _node) = dictionary.dict.find_word(&code_points);
    if terminal_id != kb_core::NOT_A_TERMINAL_ID {
        let mut prev_ids: Vec<i32> = Vec::new();
        for previous_word in previous.iter() {
            let (prev_terminal, _node) = dictionary.dict.find_word(previous_word);
            if prev_terminal == kb_core::NOT_A_TERMINAL_ID {
                break;
            }
            prev_ids.push(prev_terminal);
        }
        return dictionary
            .dict
            .word_attributes(&prev_ids, terminal_id)
            .probability;
    }
    match dictionary.dict.user.as_ref() {
        Some(user) => user.lookup_any_context(&code_points).unwrap_or(kb_core::NOT_A_PROBABILITY),
        None => kb_core::NOT_A_PROBABILITY,
    }
}

unsafe extern "system" fn get_word_property_native(
    env: JniEnvPtr,
    _class: JClass,
    handle: JLong,
    word: JIntArray,
    _is_beginning_of_sentence: JBoolean,
    out_code_points: JIntArray,
    out_flags: JBooleanArray,
    out_probability_info: JIntArray,
    _out_ngram_prev_words: JObject,
    _out_ngram_prev_words_bos: JObject,
    _out_ngram_targets: JObject,
    _out_ngram_probability_info: JObject,
    _out_shortcut_targets: JObject,
    _out_shortcut_probabilities: JObject,
) {
    let dictionary = match dict_ref(handle) {
        Some(value) => value,
        None => return,
    };
    let code_points = read_int_array(env, word);
    write_code_points(env, out_code_points, 0, kb_core::MAX_WORD_LENGTH, &code_points);

    let (terminal_id, _node) = dictionary.dict.find_word(&code_points);
    let attributes = if terminal_id != kb_core::NOT_A_TERMINAL_ID {
        dictionary.dict.word_attributes(&[], terminal_id)
    } else {
        kb_core::dict::WordAttributes::invalid()
    };
    let user_probability = match dictionary.dict.user.as_ref() {
        Some(user) => user.lookup_any_context(&code_points),
        None => None,
    };

    let mut probability = attributes.probability;
    if let Some(value) = user_probability {
        if value > probability {
            probability = value;
        }
    }

    let flags: [JBoolean; 4] = [
        if attributes.represents_beginning_of_sentence { 1 } else { 0 },
        if attributes.is_not_a_word { 1 } else { 0 },
        if attributes.is_possibly_offensive { 1 } else { 0 },
        if attributes.is_blacklisted { 1 } else { 0 },
    ];
    env::set_boolean_array_region(env, out_flags, 0, &flags);

    let probability_info: [i32; 4] = [probability, 0, 0, 1];
    env::set_int_array_region(env, out_probability_info, 0, &probability_info);
}

unsafe extern "system" fn get_next_word_native(
    env: JniEnvPtr,
    _class: JClass,
    handle: JLong,
    token: JInt,
    out_code_points: JIntArray,
    out_is_beginning_of_sentence: JBooleanArray,
) -> JInt {
    let dictionary = match dict_ref(handle) {
        Some(value) => value,
        None => return 0,
    };
    SCRATCH.with(|scratch| {
        let mut scratch = match scratch.try_borrow_mut() {
            Ok(value) => value,
            Err(_) => return 0,
        };
        let wanted = if token <= 0 {
            scratch.next_word_cache.clear();
            scratch.next_word_start = 0;
            0usize
        } else {
            token as usize
        };
        let start = scratch.next_word_start;
        let cached = scratch.next_word_cache.len();
        if wanted < start || wanted - start >= cached {
            // Walk to the requested token again, but only keep a window of it.
            scratch.next_word_cache.clear();
            scratch.next_word_start = wanted;
            let mut collected: Vec<(Vec<i32>, i32)> = Vec::with_capacity(NEXT_WORD_WINDOW);
            let mut index = 0usize;
            dictionary.dict.for_each_word(|word, _terminal, _probability| {
                if index < wanted {
                    index += 1;
                    return true;
                }
                if collected.len() >= NEXT_WORD_WINDOW {
                    return false;
                }
                collected.push((word.to_vec(), 0));
                index += 1;
                true
            });
            scratch.next_word_cache = collected;
        }
        let offset = wanted - scratch.next_word_start;
        if offset >= scratch.next_word_cache.len() {
            return 0;
        }
        let word = scratch.next_word_cache[offset].0.clone();
        write_code_points(env, out_code_points, 0, kb_core::MAX_WORD_LENGTH, &word);
        let is_beginning_of_sentence: [JBoolean; 1] = [
            if !word.is_empty() && word[0] == kb_core::CODE_POINT_BEGINNING_OF_SENTENCE {
                1
            } else {
                0
            },
        ];
        env::set_boolean_array_region(env, out_is_beginning_of_sentence, 0, &is_beginning_of_sentence);
        (wanted + 1) as JInt
    })
}

unsafe extern "system" fn get_next_valid_code_points_native(
    env: JniEnvPtr,
    _class: JClass,
    handle: JLong,
    _traverse_session: JLong,
    input_code_points: JIntArray,
    input_size: JInt,
    out_valid_code_points: JIntArray,
) {
    let dictionary = match dict_ref(handle) {
        Some(value) => value,
        None => return,
    };
    if input_size <= 0 {
        return;
    }
    let mut input = vec![0i32; input_size as usize];
    if !read_int_array_into(env, input_code_points, &mut input) {
        return;
    }

    let root = kb_core::ptrie::Node {
        pos: 0,
        children_pos: 0,
        ..kb_core::ptrie::Node::empty()
    };
    let mut current = root;
    let mut found = true;
    for code_point in input.iter() {
        match dictionary.dict.find_child(&current, *code_point) {
            Some(child) => current = child,
            None => {
                found = false;
                break;
            }
        }
    }
    if !found {
        return;
    }

    let mut out: Vec<i32> = Vec::with_capacity(64);
    if current.is_terminal() {
        out.push(' ' as i32);
    }
    dictionary.dict.for_each_child(&current, |child| {
        if child.char_count > 0 {
            out.push(child.char_at(0));
        }
        out.len() < 64
    });
    let length = out.len().min(env::array_length(env, out_valid_code_points));
    env::set_int_array_region(env, out_valid_code_points, 0, &out[..length]);
}

unsafe extern "system" fn get_suggestions_native(
    env: JniEnvPtr,
    _class: JClass,
    handle: JLong,
    proximity_handle: JLong,
    _traverse_session: JLong,
    x_coordinates: JIntArray,
    y_coordinates: JIntArray,
    times: JIntArray,
    pointer_ids: JIntArray,
    input_code_points: JIntArray,
    input_size: JInt,
    suggest_options: JIntArray,
    prev_word_arrays: JObjectArray,
    _is_beginning_of_sentence: JBooleanArray,
    prev_word_count: JInt,
    out_suggestion_count: JIntArray,
    out_code_points: JIntArray,
    out_scores: JIntArray,
    out_space_indices: JIntArray,
    out_types: JIntArray,
    out_auto_commit: JIntArray,
    inout_weight: JFloatArray,
) {
    env::set_int_array_region(env, out_suggestion_count, 0, &[0]);

    let dictionary = match dict_ref(handle) {
        Some(value) => value,
        None => return,
    };
    if proximity_handle == 0 {
        return;
    }
    let proximity: &ProximityInfo = &*(proximity_handle as *const ProximityInfo);

    let count = input_size.max(0) as usize;
    let mut xs = vec![0i32; count];
    let mut ys = vec![0i32; count];
    let mut ts = vec![0i32; count];
    let mut ids = vec![0i32; count];
    if count > 0 {
        if !read_int_array_into(env, x_coordinates, &mut xs)
            || !read_int_array_into(env, y_coordinates, &mut ys)
            || !read_int_array_into(env, times, &mut ts)
            || !read_int_array_into(env, pointer_ids, &mut ids)
        {
            return;
        }
    }
    // The Java side hands over a `MAX_WORD_LENGTH` sized array that is padded
    // with `NOT_A_CODE`; only the first `input_size` entries are input. Using
    // the array length here made the search walk 48 keys on every keystroke and
    // return nothing but the typed word.
    let mut code_points = read_int_array(env, input_code_points);
    let used = (input_size.max(0) as usize).min(code_points.len());
    code_points.truncate(used);
    let options_array = read_int_array(env, suggest_options);
    let options = SuggestOptions::from_array(&options_array);
    let previous_words = read_prev_words(env, prev_word_arrays, prev_word_count.max(0) as usize);
    let mut weight = env::get_float(env, inout_weight, 0);

    SCRATCH.with(|scratch| {
        let mut scratch = match scratch.try_borrow_mut() {
            Ok(value) => value,
            Err(_) => return,
        };
        scratch.points.clear();
        for index in 0..count {
            scratch.points.push((xs[index], ys[index], ts[index], ids[index]));
        }
        scratch.code_points.clear();
        scratch.code_points.extend_from_slice(&code_points);
        scratch.prev_words.clear();
        for word in previous_words.iter() {
            scratch.prev_words.push(word.clone());
        }

        let mut results: Vec<SuggestedWord> = std::mem::take(&mut scratch.results);
        results.clear();
        suggest::get_suggestions(
            &dictionary.dict,
            proximity,
            &scratch.points,
            &scratch.code_points,
            &scratch.prev_words,
            &options,
            &mut weight,
            &mut results,
        );

        let limit = results.len().min(MAX_RESULTS);
        for index in 0..limit {
            let word = &results[index];
            write_code_points(
                env,
                out_code_points,
                index * kb_core::MAX_WORD_LENGTH,
                kb_core::MAX_WORD_LENGTH,
                word.as_slice(),
            );
        }
        let mut scores = vec![0i32; limit];
        let mut space_indices = vec![0i32; limit];
        let mut types = vec![0i32; limit];
        for index in 0..limit {
            scores[index] = results[index].score;
            space_indices[index] = results[index].index_to_partial_commit;
            types[index] = results[index].kind;
        }
        env::set_int_array_region(env, out_scores, 0, &scores);
        env::set_int_array_region(env, out_space_indices, 0, &space_indices);
        env::set_int_array_region(env, out_types, 0, &types);
        let confidence = if limit > 0 {
            results[0].auto_commit_first_word_confidence
        } else {
            0
        };
        env::set_int_array_region(env, out_auto_commit, 0, &[confidence]);
        env::set_int_array_region(env, out_suggestion_count, 0, &[limit as i32]);
        env::set_float_array_region(env, inout_weight, 0, &[weight]);
        scratch.results = results;
    });
}

unsafe extern "system" fn add_unigram_entry_native(
    env: JniEnvPtr,
    _class: JClass,
    handle: JLong,
    word: JIntArray,
    probability: JInt,
    _shortcut_target: JIntArray,
    _shortcut_probability: JInt,
    _is_beginning_of_sentence: JBoolean,
    is_not_a_word: JBoolean,
    is_possibly_offensive: JBoolean,
    _timestamp: JInt,
) -> JBoolean {
    let dictionary = match dict_mut(handle) {
        Some(value) => value,
        None => return 0,
    };
    let code_points = read_int_array(env, word);
    if code_points.is_empty() {
        return 0;
    }
    let mut flags = 0u8;
    if is_not_a_word != 0 {
        flags |= kb_core::user::FLAG_NOT_A_WORD;
    }
    if is_possibly_offensive != 0 {
        flags |= kb_core::user::FLAG_POSSIBLY_OFFENSIVE;
    }
    match dictionary.dict.user.as_mut() {
        Some(user) => {
            user.add_word(&code_points, probability, flags, 0);
            1
        }
        None => 0,
    }
}

unsafe extern "system" fn remove_unigram_entry_native(
    env: JniEnvPtr,
    _class: JClass,
    handle: JLong,
    word: JIntArray,
) -> JBoolean {
    let dictionary = match dict_mut(handle) {
        Some(value) => value,
        None => return 0,
    };
    let code_points = read_int_array(env, word);
    match dictionary.dict.user.as_mut() {
        Some(user) => {
            if user.remove_word(&code_points) {
                1
            } else {
                0
            }
        }
        None => 0,
    }
}

unsafe extern "system" fn add_ngram_entry_native(
    env: JniEnvPtr,
    _class: JClass,
    handle: JLong,
    prev_words: JObjectArray,
    _is_beginning_of_sentence: JBooleanArray,
    word: JIntArray,
    probability: JInt,
    _timestamp: JInt,
) -> JBoolean {
    let dictionary = match dict_mut(handle) {
        Some(value) => value,
        None => return 0,
    };
    let code_points = read_int_array(env, word);
    if code_points.is_empty() {
        return 0;
    }
    let previous_count = env::array_length(env, prev_words);
    let previous = read_prev_words(env, prev_words, previous_count);
    let context = user_context_hash(&previous);
    match dictionary.dict.user.as_mut() {
        Some(user) => {
            user.add_word(&code_points, probability, 0, context);
            1
        }
        None => 0,
    }
}

unsafe extern "system" fn remove_ngram_entry_native(
    env: JniEnvPtr,
    _class: JClass,
    handle: JLong,
    _prev_words: JObjectArray,
    _is_beginning_of_sentence: JBooleanArray,
    word: JIntArray,
) -> JBoolean {
    let dictionary = match dict_mut(handle) {
        Some(value) => value,
        None => return 0,
    };
    let code_points = read_int_array(env, word);
    match dictionary.dict.user.as_mut() {
        Some(user) => {
            if user.remove_word(&code_points) {
                1
            } else {
                0
            }
        }
        None => 0,
    }
}

unsafe extern "system" fn update_entries_for_word_with_ngram_context_native(
    env: JniEnvPtr,
    _class: JClass,
    handle: JLong,
    prev_words: JObjectArray,
    _is_beginning_of_sentence: JBooleanArray,
    word: JIntArray,
    is_valid_word: JBoolean,
    count: JInt,
    _timestamp: JInt,
) -> JBoolean {
    let dictionary = match dict_mut(handle) {
        Some(value) => value,
        None => return 0,
    };
    let code_points = read_int_array(env, word);
    if code_points.is_empty() {
        return 0;
    }
    let previous_count = env::array_length(env, prev_words);
    let previous = read_prev_words(env, prev_words, previous_count);
    let context = user_context_hash(&previous);
    match dictionary.dict.user.as_mut() {
        Some(user) => {
            if is_valid_word != 0 {
                let probability = (64 + count.clamp(0, 191)) as i32;
                user.add_word(&code_points, probability, 0, context);
            } else {
                user.add_word(&code_points, 1, kb_core::user::FLAG_NOT_A_WORD, context);
            }
            1
        }
        None => 0,
    }
}

unsafe extern "system" fn update_entries_for_input_events_native(
    _env: JniEnvPtr,
    _class: JClass,
    _handle: JLong,
    input_events: JObjectArray,
    start_index: JInt,
) -> JInt {
    // `BinaryDictionary#updateEntriesForInputEvents` is `@UsedForTesting` (the
    // Sketch simulator). Real personalisation goes through
    // `updateEntriesForWordWithNgramContextNative`, which is implemented below.
    // Returning 0 stops that test-only loop without affecting typing.
    let _ = env::array_length(_env, input_events);
    let _ = start_index;
    0
}

unsafe extern "system" fn get_property_native(
    env: JniEnvPtr,
    _class: JClass,
    handle: JLong,
    _query: JString,
) -> JString {
    let dictionary = match dict_ref(handle) {
        Some(value) => value,
        None => return core::ptr::null_mut(),
    };
    let mut description = format!(
        "version={} headerSize={} locale={}",
        dictionary.dict.format_version(),
        dictionary.dict.header_size(),
        dictionary.dict.header().locale().unwrap_or_else(|| "?".to_string())
    );
    if let Some(user) = dictionary.dict.user.as_ref() {
        description.push_str(&format!(" userEntries={}", user.len()));
    }
    let bytes = std::ffi::CString::new(description).unwrap_or_else(|_| {
        std::ffi::CString::new("unavailable").unwrap()
    });
    env::new_string_utf(env, &bytes)
}

unsafe extern "system" fn is_corrupted_native(
    _env: JniEnvPtr,
    _class: JClass,
    handle: JLong,
) -> JBoolean {
    match dict_ref(handle) {
        Some(dictionary) => {
            if dictionary.dict.corrupted {
                1
            } else {
                0
            }
        }
        None => 0,
    }
}

unsafe extern "system" fn migrate_native(
    _env: JniEnvPtr,
    _class: JClass,
    _handle: JLong,
    _path: JString,
    _new_format_version: JLong,
) -> JBoolean {
    // Version 4 is the only format the Rust core understands, and it is the
    // format every dictionary is produced in, so there is nothing to migrate.
    0
}

unsafe extern "system" fn get_itrie_handle_native(
    _env: JniEnvPtr,
    _class: JClass,
    _handle: JLong,
    _letters: JString,
    _allow_bad_words: JBoolean,
) -> JLong {
    // The ITrie handle is only used by the unused "letter trie" experiment.
    0
}

fn user_context_hash(previous_words: &[Vec<i32>]) -> u32 {
    let mut slices: Vec<&[i32]> = Vec::with_capacity(previous_words.len());
    for word in previous_words {
        slices.push(word.as_slice());
    }
    kb_core::user::hash_context(&slices)
}

// ---------------------------------------------------------------------------
// ProximityInfo
// ---------------------------------------------------------------------------

unsafe extern "system" fn set_proximity_info_native(
    env: JniEnvPtr,
    _class: JClass,
    display_width: JInt,
    display_height: JInt,
    grid_width: JInt,
    grid_height: JInt,
    most_common_key_width: JInt,
    most_common_key_height: JInt,
    proximity_chars: JIntArray,
    key_count: JInt,
    key_x: JIntArray,
    key_y: JIntArray,
    key_widths: JIntArray,
    key_heights: JIntArray,
    key_code_points: JIntArray,
    sweet_x: JFloatArray,
    sweet_y: JFloatArray,
    sweet_radius: JFloatArray,
) -> JLong {
    let key_count = key_count.max(0) as usize;
    let mut key_x_values = vec![0i32; key_count];
    let mut key_y_values = vec![0i32; key_count];
    let mut key_width_values = vec![0i32; key_count];
    let mut key_height_values = vec![0i32; key_count];
    let mut key_code_point_values = vec![0i32; key_count];
    let mut sweet_x_values = vec![0.0f32; key_count];
    let mut sweet_y_values = vec![0.0f32; key_count];
    let mut sweet_radius_values = vec![0.0f32; key_count];
    if key_count > 0 {
        read_int_array_into(env, key_x, &mut key_x_values);
        read_int_array_into(env, key_y, &mut key_y_values);
        read_int_array_into(env, key_widths, &mut key_width_values);
        read_int_array_into(env, key_heights, &mut key_height_values);
        read_int_array_into(env, key_code_points, &mut key_code_point_values);
        read_float_array_into(env, sweet_x, &mut sweet_x_values);
        read_float_array_into(env, sweet_y, &mut sweet_y_values);
        read_float_array_into(env, sweet_radius, &mut sweet_radius_values);
    }
    let characters = read_int_array(env, proximity_chars);

    let info = ProximityInfo::new(
        display_width,
        display_height,
        grid_width,
        grid_height,
        most_common_key_width,
        most_common_key_height,
        characters,
        key_count,
        key_x_values,
        key_y_values,
        key_width_values,
        key_height_values,
        key_code_point_values,
        sweet_x_values,
        sweet_y_values,
        sweet_radius_values,
    );
    Box::into_raw(Box::new(info)) as JLong
}

fn read_float_array_into(env: JniEnvPtr, array: JFloatArray, out: &mut [f32]) {
    if array.is_null() || out.is_empty() {
        return;
    }
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = env::get_float(env, array, index);
    }
}

unsafe extern "system" fn release_proximity_info_native(
    _env: JniEnvPtr,
    _class: JClass,
    handle: JLong,
) {
    if handle == 0 {
        return;
    }
    drop(Box::from_raw(handle as *mut ProximityInfo));
}

// ---------------------------------------------------------------------------
// DicTraverseSession
//
// The Rust engine is stateless between calls, but the Java side treats a zero
// handle as "no session" and skips suggesting, so we hand out a small token.
// ---------------------------------------------------------------------------

unsafe extern "system" fn set_dic_traverse_session_native(
    _env: JniEnvPtr,
    _class: JClass,
    _locale: JString,
    _dict_size: JLong,
) -> JLong {
    Box::into_raw(Box::new(0u32)) as JLong
}

unsafe extern "system" fn init_dic_traverse_session_native(
    _env: JniEnvPtr,
    _class: JClass,
    _traverse_session: JLong,
    _dictionary: JLong,
    _previous_word: JIntArray,
    _previous_word_length: JInt,
) {
    // The previous-word context arrives with every suggestion request instead.
}

unsafe extern "system" fn release_dic_traverse_session_native(
    _env: JniEnvPtr,
    _class: JClass,
    traverse_session: JLong,
) {
    if traverse_session == 0 {
        return;
    }
    drop(Box::from_raw(traverse_session as *mut u32));
}

// ---------------------------------------------------------------------------
// BinaryDictionaryUtils
// ---------------------------------------------------------------------------

unsafe extern "system" fn create_empty_dict_file_native(
    env: JniEnvPtr,
    _class: JClass,
    path: JString,
    dict_version: JLong,
    locale: JString,
    attribute_keys: JObjectArray,
    attribute_values: JObjectArray,
) -> JBoolean {
    let path = match env::get_string(env, path) {
        Some(value) => value,
        None => return 0,
    };
    let version = if dict_version > 0 && dict_version <= u16::MAX as i64 {
        dict_version as u16
    } else {
        403
    };
    // The caller (ExpandableBinaryDictionary) hands us the header attributes it
    // will read back afterwards, so they have to end up in the file verbatim.
    let keys = read_string_array(env, attribute_keys);
    let values = read_string_array(env, attribute_values);
    let mut attributes: Vec<(String, String)> = Vec::with_capacity(keys.len() + 2);
    for (index, key) in keys.iter().enumerate() {
        if key.is_empty() {
            continue;
        }
        let value = match values.get(index) {
            Some(value) => value.clone(),
            None => String::new(),
        };
        attributes.push((key.clone(), value));
    }
    if !attributes.iter().any(|(key, _)| key == "version") {
        attributes.push(("version".to_string(), version.to_string()));
    }
    if !attributes.iter().any(|(key, _)| key == "locale") {
        attributes.push((
            "locale".to_string(),
            env::get_string(env, locale).unwrap_or_default(),
        ));
    }
    let borrowed: Vec<(&str, &str)> = attributes
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    if Dictionary::write_empty_v4(&path, version, &borrowed) {
        1
    } else {
        0
    }
}

fn read_string_array(env: JniEnvPtr, array: JObjectArray) -> Vec<String> {
    let length = env::array_length(env, array);
    let mut values = Vec::with_capacity(length.min(64));
    for index in 0..length.min(64) {
        let element = env::object_array_element(env, array, index);
        if element.is_null() {
            values.push(String::new());
            continue;
        }
        let value = env::get_string(env, element as JString).unwrap_or_default();
        env::delete_local_ref(env, element);
        values.push(value);
    }
    values
}

unsafe extern "system" fn calc_normalized_score_native(
    env: JniEnvPtr,
    _class: JClass,
    before: JIntArray,
    after: JIntArray,
    score: JInt,
) -> f32 {
    let before = read_int_array(env, before);
    let after = read_int_array(env, after);
    if after.is_empty() {
        return 0.0f32;
    }
    let distance = levenshtein(&before, &after);
    let length = after.len().max(1) as f32;
    let normalized = 1.0f32 - (distance as f32 / length);
    let result = (score as f32) * normalized.max(0.0f32);
    result
}

fn levenshtein(a: &[i32], b: &[i32]) -> usize {
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current: Vec<usize> = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        current[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            let deletion = previous[j] + 1;
            let insertion = current[j - 1] + 1;
            let substitution = previous[j - 1] + cost;
            current[j] = deletion.min(insertion).min(substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

static CURRENT_TIME_FOR_TEST: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

unsafe extern "system" fn set_current_time_for_test_native(
    _env: JniEnvPtr,
    _class: JClass,
    current_time: JInt,
) -> JInt {
    CURRENT_TIME_FOR_TEST.swap(current_time, std::sync::atomic::Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Registration
//
// The Rust library is loaded after the C++ one and registers the same methods
// again: `RegisterNatives` replaces the previous binding, so the Rust engine
// takes over without a single change on the Java side, and simply failing to
// load this library leaves the C++ engine in charge.
// ---------------------------------------------------------------------------

const BINARY_DICTIONARY: &str = "org/futo/inputmethod/latin/BinaryDictionary";
const PROXIMITY_INFO: &str = "org/futo/inputmethod/keyboard/ProximityInfo";
const DIC_TRAVERSE_SESSION: &str = "org/futo/inputmethod/latin/DicTraverseSession";
const BINARY_DICTIONARY_UTILS: &str = "org/futo/inputmethod/latin/utils/BinaryDictionaryUtils";

fn register(
    env_ptr: JniEnvPtr,
    class_name: &str,
    name: &str,
    signature: &str,
    function: *mut c_void,
) {
    if !env::register_one(env_ptr, class_name, name, signature, function) {
        log_error!("cannot register {}::{}{}", class_name, name, signature);
    }
}

fn register_binary_dictionary(e: JniEnvPtr) {
    let c = BINARY_DICTIONARY;
    register(e, c, "openNative", "(Ljava/lang/String;JJZ)J", open_native as *mut c_void);
    register(
        e,
        c,
        "createOnMemoryNative",
        "(JLjava/lang/String;[Ljava/lang/String;[Ljava/lang/String;)J",
        create_on_memory_native as *mut c_void,
    );
    register(
        e,
        c,
        "getHeaderInfoNative",
        "(J[I[ILjava/util/ArrayList;Ljava/util/ArrayList;)V",
        get_header_info_native as *mut c_void,
    );
    register(e, c, "flushNative", "(JLjava/lang/String;)Z", flush_native as *mut c_void);
    register(e, c, "needsToRunGCNative", "(JZ)Z", needs_to_run_gc_native as *mut c_void);
    register(
        e,
        c,
        "flushWithGCNative",
        "(JLjava/lang/String;)Z",
        flush_with_gc_native as *mut c_void,
    );
    register(e, c, "closeNative", "(J)V", close_native as *mut c_void);
    register(
        e,
        c,
        "getFormatVersionNative",
        "(J)I",
        get_format_version_native as *mut c_void,
    );
    register(
        e,
        c,
        "getProbabilityNative",
        "(J[I)I",
        get_probability_native as *mut c_void,
    );
    register(
        e,
        c,
        "getMaxProbabilityOfExactMatchesNative",
        "(J[I)I",
        get_max_probability_of_exact_matches_native as *mut c_void,
    );
    register(
        e,
        c,
        "getNgramProbabilityNative",
        "(J[[I[Z[I)I",
        get_ngram_probability_native as *mut c_void,
    );
    register(
        e,
        c,
        "getWordPropertyNative",
        "(J[IZ[I[Z[ILjava/util/ArrayList;Ljava/util/ArrayList;Ljava/util/ArrayList;Ljava/util/ArrayList;Ljava/util/ArrayList;Ljava/util/ArrayList;)V",
        get_word_property_native as *mut c_void,
    );
    register(
        e,
        c,
        "getNextWordNative",
        "(JI[I[Z)I",
        get_next_word_native as *mut c_void,
    );
    register(
        e,
        c,
        "getNextValidCodePointsNative",
        "(JJ[II[I)V",
        get_next_valid_code_points_native as *mut c_void,
    );
    register(
        e,
        c,
        "getSuggestionsNative",
        "(JJJ[I[I[I[I[II[I[[I[ZI[I[I[I[I[I[I[F)V",
        get_suggestions_native as *mut c_void,
    );
    register(
        e,
        c,
        "addUnigramEntryNative",
        "(J[II[IIZZZI)Z",
        add_unigram_entry_native as *mut c_void,
    );
    register(
        e,
        c,
        "removeUnigramEntryNative",
        "(J[I)Z",
        remove_unigram_entry_native as *mut c_void,
    );
    register(
        e,
        c,
        "addNgramEntryNative",
        "(J[[I[Z[III)Z",
        add_ngram_entry_native as *mut c_void,
    );
    register(
        e,
        c,
        "removeNgramEntryNative",
        "(J[[I[Z[I)Z",
        remove_ngram_entry_native as *mut c_void,
    );
    register(
        e,
        c,
        "updateEntriesForWordWithNgramContextNative",
        "(J[[I[Z[IZII)Z",
        update_entries_for_word_with_ngram_context_native as *mut c_void,
    );
    register(
        e,
        c,
        "updateEntriesForInputEventsNative",
        "(J[Lorg/futo/inputmethod/latin/utils/WordInputEventForPersonalization;I)I",
        update_entries_for_input_events_native as *mut c_void,
    );
    register(
        e,
        c,
        "getPropertyNative",
        "(JLjava/lang/String;)Ljava/lang/String;",
        get_property_native as *mut c_void,
    );
    register(e, c, "isCorruptedNative", "(J)Z", is_corrupted_native as *mut c_void);
    register(
        e,
        c,
        "migrateNative",
        "(JLjava/lang/String;J)Z",
        migrate_native as *mut c_void,
    );
    register(
        e,
        c,
        "getITrieHandleNative",
        "(JLjava/lang/String;Z)J",
        get_itrie_handle_native as *mut c_void,
    );
}

fn register_proximity_info(e: JniEnvPtr) {
    register(
        e,
        PROXIMITY_INFO,
        "setProximityInfoNative",
        "(IIIIII[II[I[I[I[I[I[F[F[F)J",
        set_proximity_info_native as *mut c_void,
    );
    register(
        e,
        PROXIMITY_INFO,
        "releaseProximityInfoNative",
        "(J)V",
        release_proximity_info_native as *mut c_void,
    );
}

fn register_dic_traverse_session(e: JniEnvPtr) {
    register(
        e,
        DIC_TRAVERSE_SESSION,
        "setDicTraverseSessionNative",
        "(Ljava/lang/String;J)J",
        set_dic_traverse_session_native as *mut c_void,
    );
    register(
        e,
        DIC_TRAVERSE_SESSION,
        "initDicTraverseSessionNative",
        "(JJ[II)V",
        init_dic_traverse_session_native as *mut c_void,
    );
    register(
        e,
        DIC_TRAVERSE_SESSION,
        "releaseDicTraverseSessionNative",
        "(J)V",
        release_dic_traverse_session_native as *mut c_void,
    );
}

fn register_binary_dictionary_utils(e: JniEnvPtr) {
    register(
        e,
        BINARY_DICTIONARY_UTILS,
        "createEmptyDictFileNative",
        "(Ljava/lang/String;JLjava/lang/String;[Ljava/lang/String;[Ljava/lang/String;)Z",
        create_empty_dict_file_native as *mut c_void,
    );
    register(
        e,
        BINARY_DICTIONARY_UTILS,
        "calcNormalizedScoreNative",
        "([I[II)F",
        calc_normalized_score_native as *mut c_void,
    );
    register(
        e,
        BINARY_DICTIONARY_UTILS,
        "setCurrentTimeForTestNative",
        "(I)I",
        set_current_time_for_test_native as *mut c_void,
    );
}

#[no_mangle]
pub unsafe extern "system" fn JNI_OnLoad(vm: JavaVmPtr, _reserved: *mut c_void) -> JInt {
    let env_ptr = match env::get_env_from_vm(vm) {
        Some(value) => value,
        None => {
            log_error!("JNI_OnLoad: cannot obtain JNIEnv");
            return -1;
        }
    };
    log_info!(
        "kb-core {} registered (JNI {:#x})",
        env!("CARGO_PKG_VERSION"),
        env::get_version(env_ptr)
    );
    register_binary_dictionary(env_ptr);
    register_proximity_info(env_ptr);
    register_dic_traverse_session(env_ptr);
    register_binary_dictionary_utils(env_ptr);
    env::JNI_VERSION_1_6
}
