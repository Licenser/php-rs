extern crate php_sys;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_uchar, c_void};
use std::ptr;
use std::slice;

pub struct Runtime<T> {
    callbacks: Callbacks<T>,
}

impl<T> Runtime<T> {
    pub fn new(name: &str, long_name: &str) -> RuntimeBuilder<T> {
        unsafe {
            php_sys::tsrm_startup(1, 1, 0, ptr::null_mut());
            php_sys::ts_resource_ex(0, ptr::null_mut());
            php_sys::zend_tsrmls_cache_update();

            php_sys::zend_signal_startup();

            let mut module = Box::new(php_sys::sapi_module_struct::default());
            let name = CString::new(name).unwrap();
            let pretty_name = CString::new(long_name).unwrap();

            module.name = name.into_raw();
            module.pretty_name = pretty_name.into_raw();

            module.startup = Some(sapi_server_startup::<T>);
            module.shutdown = Some(sapi_server_shutdown::<T>);
            module.ub_write = Some(sapi_server_ub_write::<T>);
            module.flush = Some(sapi_server_flush);
            module.sapi_error = Some(php_sys::zend_error);
            module.send_headers = Some(sapi_server_send_headers::<T>);
            module.read_post = Some(sapi_server_read_post::<T>);
            module.read_cookies = Some(sapi_server_read_cookies::<T>);
            module.register_server_variables = Some(sapi_server_register_variables::<T>);
            module.log_message = Some(sapi_server_log_message::<T>);
            RuntimeBuilder {
                callbacks: Callbacks::<T> {
                    startup: None,
                    shutdown: None,
                    write: None,
                    read: None,
                },
                module,
            }
        }
    }

    pub fn execute(&mut self, handle_filename: &str, context: T) -> Result<&T, ()> {
        let mode = CString::new("rb").unwrap();
        unsafe {
            php_sys::ts_resource_ex(0, ptr::null_mut());

            let fp = php_sys::phprpm_fopen(handle_filename.as_ptr() as *const i8, mode.as_ptr());
            let mut handle = php_sys::_zend_file_handle__bindgen_ty_1::default();
            handle.fp = fp;
            let script = Box::new(php_sys::zend_file_handle {
                handle: handle,
                filename: handle_filename.as_ptr() as *const i8,
                opened_path: ptr::null_mut(),
                type_: php_sys::zend_stream_type_ZEND_HANDLE_FP,
                free_filename: 0,
            });
            let script_ptr = Box::into_raw(script);

            (*php_sys::sg_sapi_headers()).http_response_code = 200;
            let context = Box::new(PHPContext {
                callbacks: &mut self.callbacks,
                context,
            });
            let context_ptr = Box::into_raw(context);
            php_sys::sg_set_server_context(context_ptr as *mut c_void);
            php_sys::php_request_startup();

            php_sys::php_execute_script(script_ptr);

            drop(Box::from_raw(script_ptr));

            let context = php_sys::sg_server_context() as *mut PHPContext<T>;
            let result = &(*context).context;

            // TODO move this into the request shutdown block
            // note: strangely enough, php_request_shutdown will not call our request shutdown callback
            if !(*php_sys::sg_request_info()).cookie_data.is_null() {
                drop(CString::from_raw((*php_sys::sg_request_info()).cookie_data));
            }
            (*php_sys::sg_request_info()).cookie_data = ptr::null_mut();
            drop(Box::from_raw(
                php_sys::sg_server_context() as *mut PHPContext<T>
            ));
            php_sys::sg_set_server_context(ptr::null_mut());

            php_sys::php_request_shutdown(ptr::null_mut());

            Ok(result)
        }
    }
}

struct PHPContext<'ctx, T: 'ctx> {
    callbacks: &'ctx mut Callbacks<T>,
    context: T,
}

pub type Module = php_sys::_sapi_module_struct;

pub struct RuntimeBuilder<T> {
    callbacks: Callbacks<T>,
    module: Box<php_sys::_sapi_module_struct>,
}

type StartupCallback<T> = FnMut(&mut T) -> Result<(), ()>;
type ShutdownCallback<T> = FnMut(&mut T) -> Result<(), ()>;
type WriteCallback<T> = FnMut(&mut T, &[u8]) -> Result<usize, ()>;
type ReadCallback<T> = FnMut(&mut T, *mut i8, usize) -> Result<usize, ()>;
struct Callbacks<T> {
    startup: Option<Box<StartupCallback<T>>>,
    shutdown: Option<Box<ShutdownCallback<T>>>,
    write: Option<Box<WriteCallback<T>>>,
    read: Option<Box<ReadCallback<T>>>,
}

impl<T> RuntimeBuilder<T> {
    pub fn startup(mut self, callback: Box<StartupCallback<T>>) -> Self {
        self.callbacks.startup = Some(callback);
        self
    }
    pub fn shutdown(mut self, callback: Box<ShutdownCallback<T>>) -> Self {
        self.callbacks.shutdown = Some(callback);
        self
    }
    pub fn write(mut self, callback: Box<WriteCallback<T>>) -> Self {
        self.callbacks.write = Some(callback);
        self
    }
    pub fn read(mut self, callback: Box<ReadCallback<T>>) -> Self {
        self.callbacks.read = Some(callback);
        self
    }
    pub fn start(self) -> Runtime<T> {
        unsafe {
            let module_ptr = Box::into_raw(self.module);
            php_sys::sapi_startup(module_ptr);

            let request_method = CString::new("POST").unwrap();
            let path_translated = CString::new("/some/file").unwrap();
            let content_type = CString::new("text/html").unwrap();
            (*php_sys::sg_request_info()).request_method = request_method.as_ptr();
            (*php_sys::sg_request_info()).content_length = 0;
            (*php_sys::sg_request_info()).path_translated = path_translated.into_raw();
            (*php_sys::sg_request_info()).content_type = content_type.as_ptr();

            php_sys::php_module_startup(module_ptr, ptr::null_mut(), 0);
        }
        Runtime {
            callbacks: self.callbacks,
        }
    }
}

unsafe extern "C" fn sapi_server_startup<T>(_module: *mut php_sys::sapi_module_struct) -> c_int {
    let context = php_sys::sg_server_context() as *mut PHPContext<T>;
    if let Some(ref mut cb) = (*context).callbacks.startup {
        if cb(&mut (*context).context).is_ok() {
            php_sys::ZEND_RESULT_CODE_SUCCESS as c_int
        } else {
            php_sys::ZEND_RESULT_CODE_FAILURE as c_int
        }
    } else {
        php_sys::ZEND_RESULT_CODE_SUCCESS as c_int
    }
}

unsafe extern "C" fn sapi_server_shutdown<T>(_module: *mut php_sys::sapi_module_struct) -> c_int {
    let context = php_sys::sg_server_context() as *mut PHPContext<T>;
    if let Some(ref mut cb) = (*context).callbacks.shutdown {
        if cb(&mut (*context).context).is_ok() {
            php_sys::ZEND_RESULT_CODE_SUCCESS as c_int
        } else {
            php_sys::ZEND_RESULT_CODE_FAILURE as c_int
        }
    } else {
        php_sys::ZEND_RESULT_CODE_SUCCESS as c_int
    }
}

unsafe extern "C" fn sapi_server_ub_write<T>(s: *const c_char, s_len: usize) -> usize {
    let context = php_sys::sg_server_context() as *mut PHPContext<T>;
    if let Some(ref mut cb) = (*context).callbacks.write {
        let s_: *const c_uchar = s as *const c_uchar;
        let rs = slice::from_raw_parts(s_, s_len);
        if let Ok(size) = cb(&mut (*context).context, rs) {
            size
        } else {
            0
        }
    } else {
        0
    }
}

unsafe extern "C" fn sapi_server_flush(_server_context: *mut c_void) {
    if php_sys::sg_headers_sent() == 1 {
        return;
    }
    php_sys::sapi_send_headers();
}

unsafe extern "C" fn sapi_server_send_headers<T>(
    _sapi_headers: *mut php_sys::sapi_headers_struct,
) -> c_int {
    // TODO
    // used so flush can try and send headers prior to output
    php_sys::sg_set_headers_sent(1);

    // bindgen treats this as a `c_uint` type but this function requires a c_int
    php_sys::SAPI_HEADER_SENT_SUCCESSFULLY as c_int
}

unsafe extern "C" fn sapi_server_read_post<T>(buf: *mut c_char, bytes: usize) -> usize {
    let context = php_sys::sg_server_context() as *mut PHPContext<T>;
    if let Some(ref mut cb) = (*context).callbacks.read {
        if let Ok(copied) = cb(&mut (*context).context, buf as *mut i8, bytes) {
            copied
        } else {
            0
        }
    } else {
        0
    }
}

unsafe extern "C" fn sapi_server_read_cookies<T>() -> *mut c_char {
    //TODO
    ptr::null_mut()
}

unsafe extern "C" fn sapi_server_register_variables<T>(_track_vars_array: *mut php_sys::zval) {
    //TODO
}

unsafe extern "C" fn sapi_server_log_message<T>(_ebmessage: *mut c_char, _syslog_type_int: c_int) {
    //TODO
}
