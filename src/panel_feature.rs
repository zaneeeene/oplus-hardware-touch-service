//! OPlus display-panel capability service.
//!
//! OPlus SystemUI asks
//! `vendor.oplus.hardware.displaypanelfeature.IDisplayPanelFeature/default`
//! for feature 217 before it enters panoramic/full-screen AOD. When that
//! service is absent, SystemUI treats panoramic AOD as energy-saving AOD and
//! hides it after a few seconds even when the panel doze level remains set.
//!
//! The vendor AIDL stubs are not part of the public NDK. This module therefore
//! uses the stable NDK binder ABI through `libbinder_ndk.so` directly. It keeps
//! the same stable-AIDL V1 transaction layout as the reference service.

#![cfg_attr(not(target_os = "android"), allow(dead_code, unused_imports))]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use log::{debug, info, warn};

use crate::config::AodPanelServiceConfig;
use crate::error::{TouchError, TouchResult};

const PANEL_DESCRIPTOR: &str = "vendor.oplus.hardware.displaypanelfeature.IDisplayPanelFeature";
const PANEL_FEATURE_AOD_CAPS: i32 = 217;
const PANEL_FEATURE_UDFPS_TYPE: i32 = 211;

// Behaviours SystemUI implements itself: doze-direct, smooth transition,
// panoramic, and panoramic all-day. The panel cannot execute feature 217's
// vendor "set" transition command, so this is the same safe set used by the
// working OPlus sensor bridge.
const AOD_PANEL_CAPABILITIES: i32 = 1 | 2 | 4 | 8;
const UDFPS_LOCAL_HBM: i32 = 16;

const AIDL_VERSION: i32 = 1;
const AIDL_HASH: &str = "39afca22ac77253421ad681eeb2bae0c6ff62c13";
const FIRST_CALL_TRANSACTION: u32 = 1;
const TRANSACTION_GET_FEATURE_VALUE: u32 = FIRST_CALL_TRANSACTION;
const TRANSACTION_SET_FEATURE_VALUE: u32 = FIRST_CALL_TRANSACTION + 1;
const TRANSACTION_GET_INFO: u32 = FIRST_CALL_TRANSACTION + 2;
const TRANSACTION_GET_INTERFACE_VERSION: u32 = FIRST_CALL_TRANSACTION + 16_777_214;
const TRANSACTION_GET_INTERFACE_HASH: u32 = FIRST_CALL_TRANSACTION + 16_777_213;

const STATUS_OK: c_int = 0;
const STATUS_ALREADY_EXISTS: c_int = -libc::EEXIST;
const STATUS_INVALID_OPERATION: c_int = -libc::ENOSYS;
const STATUS_UNKNOWN_TRANSACTION: c_int = -libc::EBADMSG;
const MAX_PANEL_VALUES: i32 = 64;

type AIBinder = c_void;
type AIBinderClass = c_void;
type AStatus = c_void;
type AParcel = c_void;

type ClassDefine = unsafe extern "C" fn(
    *const c_char,
    Option<unsafe extern "C" fn(*mut c_void) -> *mut c_void>,
    Option<unsafe extern "C" fn(*mut c_void)>,
    Option<Transact>,
) -> *mut AIBinderClass;
type BinderNew = unsafe extern "C" fn(*const AIBinderClass, *mut c_void) -> *mut AIBinder;
type Transact = unsafe extern "C" fn(*mut AIBinder, u32, *const AParcel, *mut AParcel) -> c_int;
type AddService = unsafe extern "C" fn(*mut AIBinder, *const c_char) -> c_int;
type SetThreadCount = unsafe extern "C" fn(u32);
type StartThreadPool = unsafe extern "C" fn();
type ParcelReadI32 = unsafe extern "C" fn(*const AParcel, *mut i32) -> c_int;
type ParcelWriteI32 = unsafe extern "C" fn(*mut AParcel, i32) -> c_int;
type ParcelWriteI32Array = unsafe extern "C" fn(*mut AParcel, *const i32, i32) -> c_int;
type ParcelWriteString = unsafe extern "C" fn(*mut AParcel, *const c_char, i32) -> c_int;
type WriteStatusHeader = unsafe extern "C" fn(*mut AParcel, *const AStatus) -> c_int;
type NewOkStatus = unsafe extern "C" fn() -> *mut AStatus;
type DeleteStatus = unsafe extern "C" fn(*mut AStatus);

#[derive(Clone, Copy)]
struct BinderApi {
    class_define: ClassDefine,
    binder_new: BinderNew,
    add_service: AddService,
    set_thread_count: SetThreadCount,
    start_thread_pool: StartThreadPool,
    parcel_read_i32: ParcelReadI32,
    parcel_write_i32: ParcelWriteI32,
    parcel_write_i32_array: ParcelWriteI32Array,
    parcel_write_string: ParcelWriteString,
    write_status_header: WriteStatusHeader,
    new_ok_status: NewOkStatus,
    delete_status: DeleteStatus,
}

static BINDER_API: OnceLock<BinderApi> = OnceLock::new();
// AIBinder_Class_define and AIBinder_new return owned references with process
// lifetime here. Keep them reachable so the registered service cannot rely on
// an accidental leak after the registration thread exits.
static PANEL_BINDER_CLASS: AtomicPtr<AIBinderClass> = AtomicPtr::new(ptr::null_mut());
static PANEL_BINDER: AtomicPtr<AIBinder> = AtomicPtr::new(ptr::null_mut());

struct LoadedBinder {
    api: BinderApi,
    _handle: *mut c_void,
}

pub fn start(config: AodPanelServiceConfig) -> TouchResult<()> {
    if !config.enabled {
        info!("Display panel capability service is disabled in config");
        return Ok(());
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = config;
        return Err(TouchError::NotSupported(
            "display panel binder service is Android-only".to_string(),
        ));
    }

    #[cfg(target_os = "android")]
    {
        thread::Builder::new()
            .name("oplus-panel-feature".to_string())
            .spawn(move || register_service(&config))
            .map_err(TouchError::Io)?;
        Ok(())
    }
}

#[cfg(target_os = "android")]
fn register_service(config: &AodPanelServiceConfig) {
    let Some(loaded) = load_binder() else {
        warn!("Display panel capability service disabled: libbinder_ndk.so is unavailable");
        return;
    };
    let api = loaded.api;
    if BINDER_API.set(api).is_err() {
        debug!("Display panel binder API was already initialized");
    }

    let descriptor = match CString::new(PANEL_DESCRIPTOR) {
        Ok(descriptor) => descriptor,
        Err(_) => return,
    };
    // AIBinder_Class_define keeps this interface descriptor for the class'
    // lifetime, so it must outlive this function.
    let descriptor = descriptor.into_raw();
    let service_name = match CString::new(config.service_name.as_str()) {
        Ok(service_name) => service_name,
        Err(_) => {
            warn!("Display panel service name contains a NUL byte");
            return;
        }
    };

    let class = unsafe {
        (api.class_define)(
            descriptor,
            Some(panel_on_create),
            Some(panel_on_destroy),
            Some(panel_on_transact),
        )
    };
    if class.is_null() {
        warn!("Could not define the display panel AIDL binder class");
        return;
    }

    let binder = unsafe { (api.binder_new)(class, ptr::null_mut()) };
    if binder.is_null() {
        warn!("Could not create the display panel binder service");
        return;
    }

    // Do not call AIBinder_markVintfStability here. The reference service is
    // compiled without BINDER_STABILITY_SUPPORT: marking the binder VINTF
    // stable would require a matching VINTF manifest entry. Registering the
    // local binder through servicemanager is the compatible path.
    PANEL_BINDER_CLASS.store(class, Ordering::Release);
    PANEL_BINDER.store(binder, Ordering::Release);

    unsafe {
        (api.set_thread_count)(2);
        (api.start_thread_pool)();
    }

    // Service manager is normally up by late-fs. Retry transiently without
    // restarting the daemon if this service was started unusually early.
    for attempt in 0..30 {
        let status = unsafe { (api.add_service)(binder, service_name.as_ptr()) };
        match status {
            STATUS_OK => {
                info!(
                    "Registered display panel capability service {}",
                    config.service_name
                );
                return;
            }
            STATUS_ALREADY_EXISTS => {
                info!(
                    "Display panel capability service {} is already registered",
                    config.service_name
                );
                return;
            }
            STATUS_INVALID_OPERATION if attempt < 29 => {
                thread::sleep(Duration::from_secs(1));
            }
            other => {
                if attempt == 29 {
                    warn!(
                        "Could not register display panel service {}: status {other}",
                        config.service_name
                    );
                } else {
                    thread::sleep(Duration::from_secs(1));
                }
            }
        }
    }
}

fn load_binder() -> Option<LoadedBinder> {
    let handle = unsafe {
        libc::dlopen(
            c"libbinder_ndk.so".as_ptr(),
            libc::RTLD_NOW | libc::RTLD_LOCAL,
        )
    };
    if handle.is_null() {
        return None;
    }

    unsafe {
        Some(LoadedBinder {
            api: BinderApi {
                class_define: binder_symbol(handle, c"AIBinder_Class_define")?,
                binder_new: binder_symbol(handle, c"AIBinder_new")?,
                add_service: binder_symbol(handle, c"AServiceManager_addService")?,
                set_thread_count: binder_symbol(
                    handle,
                    c"ABinderProcess_setThreadPoolMaxThreadCount",
                )?,
                start_thread_pool: binder_symbol(handle, c"ABinderProcess_startThreadPool")?,
                parcel_read_i32: binder_symbol(handle, c"AParcel_readInt32")?,
                parcel_write_i32: binder_symbol(handle, c"AParcel_writeInt32")?,
                parcel_write_i32_array: binder_symbol(handle, c"AParcel_writeInt32Array")?,
                parcel_write_string: binder_symbol(handle, c"AParcel_writeString")?,
                write_status_header: binder_symbol(handle, c"AParcel_writeStatusHeader")?,
                new_ok_status: binder_symbol(handle, c"AStatus_newOk")?,
                delete_status: binder_symbol(handle, c"AStatus_delete")?,
            },
            _handle: handle,
        })
    }
}

unsafe fn binder_symbol<T: Copy>(handle: *mut c_void, name: &CStr) -> Option<T> {
    let symbol = unsafe { libc::dlsym(handle, name.as_ptr()) };
    if symbol.is_null() {
        warn!(
            "Display panel service requires missing binder symbol {}",
            name.to_string_lossy()
        );
        None
    } else {
        Some(unsafe { std::mem::transmute_copy(&symbol) })
    }
}

unsafe extern "C" fn panel_on_create(_args: *mut c_void) -> *mut c_void {
    Box::into_raw(Box::new(0u8)).cast()
}

unsafe extern "C" fn panel_on_destroy(user_data: *mut c_void) {
    if !user_data.is_null() {
        drop(unsafe { Box::from_raw(user_data.cast::<u8>()) });
    }
}

unsafe extern "C" fn panel_on_transact(
    _binder: *mut AIBinder,
    code: u32,
    input: *const AParcel,
    output: *mut AParcel,
) -> c_int {
    let Some(api) = BINDER_API.get() else {
        return STATUS_UNKNOWN_TRANSACTION;
    };

    match code {
        TRANSACTION_GET_FEATURE_VALUE => unsafe { get_feature_value(api, input, output) },
        TRANSACTION_SET_FEATURE_VALUE => unsafe { set_feature_value(api, input, output) },
        TRANSACTION_GET_INFO => STATUS_UNKNOWN_TRANSACTION,
        TRANSACTION_GET_INTERFACE_VERSION => unsafe {
            write_status_and_int(api, output, AIDL_VERSION)
        },
        TRANSACTION_GET_INTERFACE_HASH => unsafe {
            if let Some(status) = write_ok_header(api, output) {
                return status;
            }
            (api.parcel_write_string)(output, AIDL_HASH.as_ptr().cast(), AIDL_HASH.len() as i32)
        },
        _ => STATUS_UNKNOWN_TRANSACTION,
    }
}

unsafe fn get_feature_value(api: &BinderApi, input: *const AParcel, output: *mut AParcel) -> c_int {
    let mut feature = 0;
    if unsafe { (api.parcel_read_i32)(input, &mut feature) } != STATUS_OK {
        return STATUS_UNKNOWN_TRANSACTION;
    }

    let value = match feature {
        PANEL_FEATURE_AOD_CAPS => AOD_PANEL_CAPABILITIES,
        PANEL_FEATURE_UDFPS_TYPE => UDFPS_LOCAL_HBM,
        _ => return STATUS_UNKNOWN_TRANSACTION,
    };

    // `out int[] values` is pre-sized by the client. The generated AIDL
    // proxy sends only that size before the transaction.
    let mut length = 0;
    if unsafe { (api.parcel_read_i32)(input, &mut length) } != STATUS_OK
        || !(1..=MAX_PANEL_VALUES).contains(&length)
    {
        return STATUS_UNKNOWN_TRANSACTION;
    }

    let mut values = vec![0; length as usize];
    values[0] = value;
    unsafe { write_feature_value_response(api, output, &values) }
}

unsafe fn set_feature_value(api: &BinderApi, input: *const AParcel, output: *mut AParcel) -> c_int {
    let mut feature = 0;
    if unsafe { (api.parcel_read_i32)(input, &mut feature) } != STATUS_OK {
        return STATUS_UNKNOWN_TRANSACTION;
    }
    if feature != PANEL_FEATURE_AOD_CAPS {
        return STATUS_UNKNOWN_TRANSACTION;
    }

    unsafe { write_status_and_int(api, output, 0) }
}

unsafe fn write_status_and_int(api: &BinderApi, output: *mut AParcel, value: i32) -> c_int {
    if let Some(status) = unsafe { write_ok_header(api, output) } {
        return status;
    }
    unsafe { (api.parcel_write_i32)(output, value) }
}

unsafe fn write_feature_value_response(
    api: &BinderApi,
    output: *mut AParcel,
    values: &[i32],
) -> c_int {
    if let Some(status) = unsafe { write_ok_header(api, output) } {
        return status;
    }

    // Stable AIDL returns `int getDisplayPanelFeatureValue(..., out int[])`.
    // The generated Bn implementation writes the method return value before
    // the out array. Omitting this zero makes SystemUI reject feature 217's
    // answer and enables the temporary/energy-saving AOD path.
    let result = unsafe { (api.parcel_write_i32)(output, 0) };
    if result != STATUS_OK {
        return result;
    }
    unsafe { (api.parcel_write_i32_array)(output, values.as_ptr(), values.len() as i32) }
}

unsafe fn write_ok_header(api: &BinderApi, output: *mut AParcel) -> Option<c_int> {
    let status = unsafe { (api.new_ok_status)() };
    if status.is_null() {
        return Some(STATUS_INVALID_OPERATION);
    }
    let result = unsafe { (api.write_status_header)(output, status) };
    unsafe { (api.delete_status)(status) };
    (result != STATUS_OK).then_some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum ParcelWrite {
        StatusOk,
        Int(i32),
        IntArray(Vec<i32>),
        String(String),
    }

    #[derive(Default)]
    struct MockParcel {
        reads: std::collections::VecDeque<i32>,
        writes: Mutex<Vec<ParcelWrite>>,
    }

    unsafe extern "C" fn mock_read_i32(parcel: *const AParcel, value: *mut i32) -> c_int {
        let parcel = unsafe { &mut *(parcel.cast::<MockParcel>() as *mut MockParcel) };
        let Some(next) = parcel.reads.front().copied() else {
            return STATUS_UNKNOWN_TRANSACTION;
        };
        parcel.reads.pop_front();
        unsafe { *value = next };
        STATUS_OK
    }

    unsafe extern "C" fn mock_write_i32(parcel: *mut AParcel, value: i32) -> c_int {
        let parcel = unsafe { &*(parcel.cast::<MockParcel>()) };
        parcel
            .writes
            .lock()
            .expect("mock parcel lock")
            .push(ParcelWrite::Int(value));
        STATUS_OK
    }

    unsafe extern "C" fn mock_write_i32_array(
        parcel: *mut AParcel,
        values: *const i32,
        length: i32,
    ) -> c_int {
        let parcel = unsafe { &*(parcel.cast::<MockParcel>()) };
        let values = unsafe { std::slice::from_raw_parts(values, length.max(0) as usize) };
        parcel
            .writes
            .lock()
            .expect("mock parcel lock")
            .push(ParcelWrite::IntArray(values.to_vec()));
        STATUS_OK
    }

    unsafe extern "C" fn mock_write_string(
        parcel: *mut AParcel,
        value: *const c_char,
        length: i32,
    ) -> c_int {
        let parcel = unsafe { &*(parcel.cast::<MockParcel>()) };
        let bytes =
            unsafe { std::slice::from_raw_parts(value.cast::<u8>(), length.max(0) as usize) };
        parcel
            .writes
            .lock()
            .expect("mock parcel lock")
            .push(ParcelWrite::String(
                String::from_utf8_lossy(bytes).into_owned(),
            ));
        STATUS_OK
    }

    unsafe extern "C" fn mock_write_status(parcel: *mut AParcel, _status: *const AStatus) -> c_int {
        let parcel = unsafe { &*(parcel.cast::<MockParcel>()) };
        parcel
            .writes
            .lock()
            .expect("mock parcel lock")
            .push(ParcelWrite::StatusOk);
        STATUS_OK
    }

    unsafe extern "C" fn mock_new_ok_status() -> *mut AStatus {
        1usize as *mut AStatus
    }

    unsafe extern "C" fn mock_delete_status(_status: *mut AStatus) {}

    unsafe extern "C" fn unused_class_define(
        _descriptor: *const c_char,
        _on_create: Option<unsafe extern "C" fn(*mut c_void) -> *mut c_void>,
        _on_destroy: Option<unsafe extern "C" fn(*mut c_void)>,
        _on_transact: Option<Transact>,
    ) -> *mut AIBinderClass {
        ptr::null_mut()
    }

    unsafe extern "C" fn unused_binder_new(
        _class: *const AIBinderClass,
        _args: *mut c_void,
    ) -> *mut AIBinder {
        ptr::null_mut()
    }

    unsafe extern "C" fn unused_add_service(_binder: *mut AIBinder, _name: *const c_char) -> c_int {
        STATUS_INVALID_OPERATION
    }

    unsafe extern "C" fn unused_set_thread_count(_count: u32) {}
    unsafe extern "C" fn unused_start_thread_pool() {}

    fn mock_api() -> BinderApi {
        BinderApi {
            class_define: unused_class_define,
            binder_new: unused_binder_new,
            add_service: unused_add_service,
            set_thread_count: unused_set_thread_count,
            start_thread_pool: unused_start_thread_pool,
            parcel_read_i32: mock_read_i32,
            parcel_write_i32: mock_write_i32,
            parcel_write_i32_array: mock_write_i32_array,
            parcel_write_string: mock_write_string,
            write_status_header: mock_write_status,
            new_ok_status: mock_new_ok_status,
            delete_status: mock_delete_status,
        }
    }

    #[test]
    fn full_screen_capabilities_match_reference() {
        assert_eq!(AOD_PANEL_CAPABILITIES, 15);
        assert_eq!(UDFPS_LOCAL_HBM, 16);
    }

    #[test]
    fn stable_aidl_transaction_numbers_are_unchanged() {
        assert_eq!(TRANSACTION_GET_FEATURE_VALUE, 1);
        assert_eq!(TRANSACTION_SET_FEATURE_VALUE, 2);
        assert_eq!(TRANSACTION_GET_INFO, 3);
        assert_eq!(TRANSACTION_GET_INTERFACE_VERSION, 16_777_215);
        assert_eq!(TRANSACTION_GET_INTERFACE_HASH, 16_777_214);
    }

    #[test]
    fn feature_response_contains_status_return_value_then_array() {
        let api = mock_api();
        let mut input = MockParcel {
            reads: [PANEL_FEATURE_AOD_CAPS, 1].into_iter().collect(),
            writes: Mutex::new(Vec::new()),
        };
        let mut output = MockParcel::default();

        let result = unsafe {
            get_feature_value(
                &api,
                (&mut input as *mut MockParcel).cast(),
                (&mut output as *mut MockParcel).cast(),
            )
        };
        assert_eq!(result, STATUS_OK);
        assert_eq!(
            output.writes.lock().expect("mock parcel lock").as_slice(),
            &[
                ParcelWrite::StatusOk,
                ParcelWrite::Int(0),
                ParcelWrite::IntArray(vec![AOD_PANEL_CAPABILITIES]),
            ]
        );
    }

    #[test]
    fn udfps_response_preserves_requested_array_length() {
        let api = mock_api();
        let mut input = MockParcel {
            reads: [PANEL_FEATURE_UDFPS_TYPE, 2].into_iter().collect(),
            writes: Mutex::new(Vec::new()),
        };
        let mut output = MockParcel::default();

        let result = unsafe {
            get_feature_value(
                &api,
                (&mut input as *mut MockParcel).cast(),
                (&mut output as *mut MockParcel).cast(),
            )
        };
        assert_eq!(result, STATUS_OK);
        assert_eq!(
            output.writes.lock().expect("mock parcel lock").as_slice(),
            &[
                ParcelWrite::StatusOk,
                ParcelWrite::Int(0),
                ParcelWrite::IntArray(vec![UDFPS_LOCAL_HBM, 0]),
            ]
        );
    }
}
