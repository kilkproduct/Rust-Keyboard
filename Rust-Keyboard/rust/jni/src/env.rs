//! Thin, typed accessors for the JNI function table.
//!
//! [`jni_sys::JNINativeInterface`] mirrors the C struct field for field; this
//! module turns the handful of entries we actually call into ordinary Rust
//! functions.  Every accessor is a `transmute_copy` of a function pointer, so
//! the only thing that can be wrong is a signature -- the *positions* come
//! straight from the vendored `jni.h` (see `rust/tools/gen_jni_sys.py`).

use core::ffi::{c_char, c_void};

use crate::jni_sys::JNINativeInterface;

pub type JniEnvPtr = *const JNINativeInterface;
pub type JavaVmPtr = *const JniInvokeInterface;
pub type JObject = *mut c_void;
pub type JClass = *mut c_void;
pub type JString = *mut c_void;
pub type JArray = *mut c_void;
pub type JIntArray = *mut c_void;
pub type JBooleanArray = *mut c_void;
pub type JFloatArray = *mut c_void;
pub type JObjectArray = *mut c_void;
pub type JLong = i64;
pub type JInt = i32;
pub type JBoolean = u8;
pub type JFloat = f32;
pub type JSize = i32;
pub type JMethodId = *mut c_void;
pub type JFieldId = *mut c_void;

pub const JNI_VERSION_1_6: JInt = 0x0001_0006;

/// A single entry of a `JNINativeMethod[]` table.
#[repr(C)]
pub struct JniNativeMethod {
    pub name: *const c_char,
    pub signature: *const c_char,
    pub fn_ptr: *mut c_void,
}

/// Mirror of `JNIInvokeInterface_` (the JavaVM function table).
///
/// Three reserved slots, then DestroyJavaVM; see `rust/tools/jni.h`. A fourth
/// reserved slot here would resolve `GetEnv` to `AttachCurrentThreadAsDaemon`.
#[repr(C)]
pub struct JniInvokeInterface {
    pub reserved0: *mut c_void,
    pub reserved1: *mut c_void,
    pub reserved2: *mut c_void,
    pub DestroyJavaVM: *mut c_void,
    pub AttachCurrentThread: *mut c_void,
    pub DetachCurrentThread: *mut c_void,
    pub GetEnv: *mut c_void,
    pub AttachCurrentThreadAsDaemon: *mut c_void,
}


/// A table of null function pointers, returned instead of dereferencing a null
/// environment. It is only ever reached if the VM hands us something broken.
struct EmptyTable(JNINativeInterface);

// The table is immutable and never written to.
unsafe impl Sync for EmptyTable {}

static EMPTY_TABLE: EmptyTable = EmptyTable(unsafe { core::mem::zeroed() });

#[inline]
fn table(env: JniEnvPtr) -> &'static JNINativeInterface {
    // A JNIEnv points at a structure whose first word is the address of the
    // function table (`(*env)->FindClass` in C). Reading the table straight out
    // of `env` makes every slot address an address inside the VM, so the first
    // call jumps into nothing - which is what crashed the app on launch.
    unsafe {
        if env.is_null() {
            return &EMPTY_TABLE.0;
        }
        let functions = *(env as *const *const JNINativeInterface);
        if functions.is_null() {
            return &EMPTY_TABLE.0;
        }
        &*functions
    }
}

// --- functions -----------------------------------------------------------------

type FnGetVersion = unsafe extern "system" fn(JniEnvPtr) -> JInt;
type FnFindClass = unsafe extern "system" fn(JniEnvPtr, *const c_char) -> JClass;
type FnGetMethodId =
    unsafe extern "system" fn(JniEnvPtr, JClass, *const c_char, *const c_char) -> JMethodId;
type FnNewObjectA =
    unsafe extern "system" fn(JniEnvPtr, JClass, JMethodId, *const c_void) -> JObject;
type FnCallBooleanMethodA =
    unsafe extern "system" fn(JniEnvPtr, JObject, JMethodId, *const c_void) -> JBoolean;
type FnCallVoidMethodA = unsafe extern "system" fn(JniEnvPtr, JObject, JMethodId, *const c_void);
type FnDeleteLocalRef = unsafe extern "system" fn(JniEnvPtr, JObject);
type FnGetArrayLength = unsafe extern "system" fn(JniEnvPtr, JArray) -> JSize;
type FnNewIntArray = unsafe extern "system" fn(JniEnvPtr, JSize) -> JIntArray;
type FnGetIntArrayRegion =
    unsafe extern "system" fn(JniEnvPtr, JIntArray, JSize, JSize, *mut JInt);
type FnSetIntArrayRegion =
    unsafe extern "system" fn(JniEnvPtr, JIntArray, JSize, JSize, *const JInt);
type FnSetBooleanArrayRegion =
    unsafe extern "system" fn(JniEnvPtr, JBooleanArray, JSize, JSize, *const JBoolean);
type FnGetFloatArrayRegion =
    unsafe extern "system" fn(JniEnvPtr, JFloatArray, JSize, JSize, *mut JFloat);
type FnSetFloatArrayRegion =
    unsafe extern "system" fn(JniEnvPtr, JFloatArray, JSize, JSize, *const JFloat);
type FnGetObjectArrayElement =
    unsafe extern "system" fn(JniEnvPtr, JObjectArray, JSize) -> JObject;
type FnGetStringUtfChars =
    unsafe extern "system" fn(JniEnvPtr, JString, *mut JBoolean) -> *const c_char;
type FnReleaseStringUtfChars = unsafe extern "system" fn(JniEnvPtr, JString, *const c_char);
type FnRegisterNatives =
    unsafe extern "system" fn(JniEnvPtr, JClass, *const JniNativeMethod, JInt) -> JInt;
type FnExceptionCheck = unsafe extern "system" fn(JniEnvPtr) -> JBoolean;
type FnExceptionClear = unsafe extern "system" fn(JniEnvPtr);

#[inline]
fn f_GetFindClass(table: &JNINativeInterface) -> FnFindClass {
    unsafe { core::mem::transmute_copy(&table.FindClass) }
}

#[inline]
fn f_GetMethodId(table: &JNINativeInterface) -> FnGetMethodId {
    unsafe { core::mem::transmute_copy(&table.GetMethodID) }
}

#[inline]
fn f_NewObjectA(table: &JNINativeInterface) -> FnNewObjectA {
    unsafe { core::mem::transmute_copy(&table.NewObjectA) }
}

#[inline]
fn f_CallBooleanMethodA(table: &JNINativeInterface) -> FnCallBooleanMethodA {
    unsafe { core::mem::transmute_copy(&table.CallBooleanMethodA) }
}

#[inline]
fn f_DeleteLocalRef(table: &JNINativeInterface) -> FnDeleteLocalRef {
    unsafe { core::mem::transmute_copy(&table.DeleteLocalRef) }
}

#[inline]
fn f_GetArrayLength(table: &JNINativeInterface) -> FnGetArrayLength {
    unsafe { core::mem::transmute_copy(&table.GetArrayLength) }
}

#[inline]
fn f_NewIntArray(table: &JNINativeInterface) -> FnNewIntArray {
    unsafe { core::mem::transmute_copy(&table.NewIntArray) }
}

#[inline]
fn f_GetIntArrayRegion(table: &JNINativeInterface) -> FnGetIntArrayRegion {
    unsafe { core::mem::transmute_copy(&table.GetIntArrayRegion) }
}

#[inline]
fn f_SetIntArrayRegion(table: &JNINativeInterface) -> FnSetIntArrayRegion {
    unsafe { core::mem::transmute_copy(&table.SetIntArrayRegion) }
}

#[inline]
fn f_SetBooleanArrayRegion(table: &JNINativeInterface) -> FnSetBooleanArrayRegion {
    unsafe { core::mem::transmute_copy(&table.SetBooleanArrayRegion) }
}

#[inline]
fn f_GetFloatArrayRegion(table: &JNINativeInterface) -> FnGetFloatArrayRegion {
    unsafe { core::mem::transmute_copy(&table.GetFloatArrayRegion) }
}

#[inline]
fn f_SetFloatArrayRegion(table: &JNINativeInterface) -> FnSetFloatArrayRegion {
    unsafe { core::mem::transmute_copy(&table.SetFloatArrayRegion) }
}

#[inline]
fn f_GetObjectArrayElement(table: &JNINativeInterface) -> FnGetObjectArrayElement {
    unsafe { core::mem::transmute_copy(&table.GetObjectArrayElement) }
}

#[inline]
fn f_GetStringUtfChars(table: &JNINativeInterface) -> FnGetStringUtfChars {
    unsafe { core::mem::transmute_copy(&table.GetStringUTFChars) }
}

#[inline]
fn f_ReleaseStringUtfChars(table: &JNINativeInterface) -> FnReleaseStringUtfChars {
    unsafe { core::mem::transmute_copy(&table.ReleaseStringUTFChars) }
}

#[inline]
fn f_RegisterNatives(table: &JNINativeInterface) -> FnRegisterNatives {
    unsafe { core::mem::transmute_copy(&table.RegisterNatives) }
}

#[inline]
fn f_ExceptionCheck(table: &JNINativeInterface) -> FnExceptionCheck {
    unsafe { core::mem::transmute_copy(&table.ExceptionCheck) }
}

#[inline]
fn f_ExceptionClear(table: &JNINativeInterface) -> FnExceptionClear {
    unsafe { core::mem::transmute_copy(&table.ExceptionClear) }
}

#[inline]
fn f_GetVersion(table: &JNINativeInterface) -> FnGetVersion {
    unsafe { core::mem::transmute_copy(&table.GetVersion) }
}

#[inline]
fn f_CallVoidMethodA(table: &JNINativeInterface) -> FnCallVoidMethodA {
    unsafe { core::mem::transmute_copy(&table.CallVoidMethodA) }
}

// --- safe wrappers -------------------------------------------------------------

#[inline]
pub fn get_version(env: JniEnvPtr) -> JInt {
    unsafe { f_GetVersion(table(env))(env) }
}

#[inline]
pub fn exception_clear(env: JniEnvPtr) {
    unsafe { f_ExceptionClear(table(env))(env) }
}

#[inline]
pub fn exception_check(env: JniEnvPtr) -> bool {
    unsafe { f_ExceptionCheck(table(env))(env) != 0 }
}

#[inline]
pub fn array_length(env: JniEnvPtr, array: JArray) -> usize {
    if array.is_null() {
        return 0;
    }
    let length = unsafe { f_GetArrayLength(table(env))(env, array) };
    if length < 0 {
        0
    } else {
        length as usize
    }
}

/// Copies `out.len()` integers starting at `start`; returns `false` on error.
#[inline]
pub fn get_int_array_region(
    env: JniEnvPtr,
    array: JIntArray,
    start: usize,
    out: &mut [JInt],
) -> bool {
    if array.is_null() || out.is_empty() {
        return false;
    }
    unsafe {
        f_GetIntArrayRegion(table(env))(
            env,
            array,
            start as JSize,
            out.len() as JSize,
            out.as_mut_ptr(),
        )
    };
    !exception_check(env)
}

#[inline]
pub fn set_int_array_region(env: JniEnvPtr, array: JIntArray, start: usize, values: &[JInt]) {
    if array.is_null() || values.is_empty() {
        return;
    }
    unsafe {
        f_SetIntArrayRegion(table(env))(
            env,
            array,
            start as JSize,
            values.len() as JSize,
            values.as_ptr(),
        )
    };
}

#[inline]
pub fn set_boolean_array_region(
    env: JniEnvPtr,
    array: JBooleanArray,
    start: usize,
    values: &[JBoolean],
) {
    if array.is_null() || values.is_empty() {
        return;
    }
    unsafe {
        f_SetBooleanArrayRegion(table(env))(
            env,
            array,
            start as JSize,
            values.len() as JSize,
            values.as_ptr(),
        )
    };
}

#[inline]
pub fn set_float_array_region(env: JniEnvPtr, array: JFloatArray, start: usize, values: &[JFloat]) {
    if array.is_null() || values.is_empty() {
        return;
    }
    unsafe {
        f_SetFloatArrayRegion(table(env))(
            env,
            array,
            start as JSize,
            values.len() as JSize,
            values.as_ptr(),
        )
    };
}

#[inline]
pub fn get_float(env: JniEnvPtr, array: JFloatArray, index: usize) -> f32 {
    if array.is_null() {
        return 0.0f32;
    }
    let mut value: [JFloat; 1] = [0.0f32];
    unsafe {
        f_GetFloatArrayRegion(table(env))(env, array, index as JSize, 1, value.as_mut_ptr());
    }
    value[0]
}

#[inline]
pub fn object_array_element(
    env: JniEnvPtr,
    array: JObjectArray,
    index: usize,
) -> JObject {
    if array.is_null() {
        return core::ptr::null_mut();
    }
    unsafe { f_GetObjectArrayElement(table(env))(env, array, index as JSize) }
}

#[inline]
pub fn delete_local_ref(env: JniEnvPtr, object: JObject) {
    if object.is_null() {
        return;
    }
    unsafe { f_DeleteLocalRef(table(env))(env, object) };
}

/// Reads a Java string into a Rust `String` (lossy, UTF-8 based).
pub fn get_string(env: JniEnvPtr, string: JString) -> Option<String> {
    if string.is_null() {
        return None;
    }
    let mut is_copy: JBoolean = 0;
    let chars = unsafe { f_GetStringUtfChars(table(env))(env, string, &mut is_copy) };
    if chars.is_null() {
        return None;
    }
    let mut length = 0usize;
    unsafe {
        while *chars.add(length) != 0 {
            length += 1;
            if length > 4096 {
                break;
            }
        }
        let bytes = core::slice::from_raw_parts(chars as *const u8, length);
        let result = String::from_utf8_lossy(bytes).into_owned();
        f_ReleaseStringUtfChars(table(env))(env, string, chars);
        Some(result)
    }
}

/// Creates an `int[]` from `values`.
pub fn new_int_array(env: JniEnvPtr, values: &[JInt]) -> JIntArray {
    let array = unsafe { f_NewIntArray(table(env))(env, values.len() as JSize) };
    if array.is_null() {
        return array;
    }
    set_int_array_region(env, array, 0, values);
    array
}

/// Appends an object to a list (`ArrayList#add(Object)`).
pub fn add_object_to_list(
    env: JniEnvPtr,
    list: JObject,
    add_method: JMethodId,
    value: JObject,
) {
    if list.is_null() || add_method.is_null() || value.is_null() {
        return;
    }
    let args: [usize; 1] = [value as usize];
    unsafe {
        f_CallBooleanMethodA(table(env))(env, list, add_method, args.as_ptr() as *const c_void);
    }
    // Every call into Java may leave an exception pending; nothing below this
    // point can do anything useful with one, and a pending exception makes the
    // next JNI call a debugging error.
    if exception_check(env) {
        exception_clear(env);
    }
}


type FnNewStringUtf = unsafe extern "system" fn(JniEnvPtr, *const c_char) -> JString;
type FnGetObjectClass = unsafe extern "system" fn(JniEnvPtr, JObject) -> JClass;

#[inline]
fn f_NewStringUtf(api: &JNINativeInterface) -> FnNewStringUtf {
    unsafe { core::mem::transmute_copy(&api.NewStringUTF) }
}

#[inline]
fn f_GetObjectClass(api: &JNINativeInterface) -> FnGetObjectClass {
    unsafe { core::mem::transmute_copy(&api.GetObjectClass) }
}

/// Wraps an already NUL-terminated Rust string into a `java.lang.String`.
#[inline]
pub fn new_string_utf(env: JniEnvPtr, value: &std::ffi::CString) -> JString {
    unsafe { f_NewStringUtf(table(env))(env, value.as_ptr()) }
}

/// Returns the `ArrayList#add` method id for an existing list instance.
pub fn add_method_of(env: JniEnvPtr, list: JObject) -> Option<JMethodId> {
    if list.is_null() {
        return None;
    }
    let api = table(env);
    let class = unsafe { f_GetObjectClass(api)(env, list) };
    if class.is_null() {
        exception_clear(env);
        return None;
    }
    let name = std::ffi::CString::new("add").ok()?;
    let signature = std::ffi::CString::new("(Ljava/lang/Object;)Z").ok()?;
    let method = unsafe { f_GetMethodId(api)(env, class, name.as_ptr(), signature.as_ptr()) };
    if method.is_null() {
        exception_clear(env);
        return None;
    }
    Some(method)
}

/// Registers exactly one native method. Doing this per method means a single
/// wrong signature cannot disable the whole bridge.
pub fn register_one(
    env: JniEnvPtr,
    class_name: &str,
    name: &str,
    signature: &str,
    function: *mut c_void,
) -> bool {
    let api = table(env);
    let class_name_c = match std::ffi::CString::new(class_name) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let class = unsafe { f_GetFindClass(api)(env, class_name_c.as_ptr()) };
    if class.is_null() {
        exception_clear(env);
        return false;
    }
    let name_c = match std::ffi::CString::new(name) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let signature_c = match std::ffi::CString::new(signature) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let methods = [JniNativeMethod {
        name: name_c.as_ptr(),
        signature: signature_c.as_ptr(),
        fn_ptr: function,
    }];
    let result = unsafe { f_RegisterNatives(api)(env, class, methods.as_ptr(), 1) };
    if result != 0 {
        exception_clear(env);
        return false;
    }
    true
}

/// Registers native methods for `class_name`.
pub fn register_natives(
    env: JniEnvPtr,
    class_name: &str,
    methods: &[JniNativeMethod],
) -> bool {
    let api = table(env);
    let name = match std::ffi::CString::new(class_name) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let class = unsafe { f_GetFindClass(api)(env, name.as_ptr()) };
    if class.is_null() {
        exception_clear(env);
        return false;
    }
    let result = unsafe {
        f_RegisterNatives(api)(env, class, methods.as_ptr(), methods.len() as JInt)
    };
    if result != 0 {
        exception_clear(env);
        return false;
    }
    true
}

/// Invokes a no-argument `void` method (used for the `StringBuilder` free path).
pub fn call_void(env: JniEnvPtr, object: JObject, method: JMethodId) {
    if object.is_null() || method.is_null() {
        return;
    }
    unsafe {
        f_CallVoidMethodA(table(env))(env, object, method, core::ptr::null::<c_void>());
    }
}

/// Extracts the `JNIEnv` from a `JavaVM` inside `JNI_OnLoad`.
pub fn get_env_from_vm(vm: JavaVmPtr) -> Option<JniEnvPtr> {
    type FnGetEnv =
        unsafe extern "system" fn(JavaVmPtr, *mut *mut c_void, JInt) -> JInt;
    if vm.is_null() {
        return None;
    }
    // `vm` is the handle the VM passed to JNI_OnLoad; like a JNIEnv it points at
    // a word holding the function table address (`(*vm)->GetEnv` in C).
    let functions = unsafe { *(vm as *const *const JniInvokeInterface) };
    if functions.is_null() {
        return None;
    }
    let api = unsafe { &*functions };
    let get_env: FnGetEnv = unsafe { core::mem::transmute_copy(&api.GetEnv) };
    let mut env: *mut c_void = core::ptr::null_mut();
    let result = unsafe { get_env(vm, &mut env, JNI_VERSION_1_6) };
    if result != 0 || env.is_null() {
        return None;
    }
    Some(env as JniEnvPtr)
}
