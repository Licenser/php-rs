//! A rust wrapper around libphp

extern crate php_sys;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_uchar, c_void};
use std::ptr;
use std::slice;

/// PHP Runtime to execute code in.
pub struct Runtime<T> {
    callbacks: Callbacks<T>,
}

impl<T> Runtime<T> {
    /// Creates a php runtime, should ever only be a single run runtime as libphp
    /// shares state.
    ///
    /// A builder is returned to set callbacks as needed.
    ///
    /// `name` - is the short name of the runtime
    /// `long_name` - is the long/descriptive name of the runtime
    /// `threads` - number of runtime threads
    pub fn new(name: &str, long_name: &str, threads: usize) -> RuntimeBuilder<T> {
        let threads = if threads > 0 { threads } else { 1 };
        unsafe {
            php_sys::tsrm_startup(threads as i32, 1, 0, ptr::null_mut());
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

    /// Executes php code, given a php file and a context. The context can be used
    /// to pass additional information to the callbacks.
    pub fn execute(&mut self, handle_filename: &str, context: T) -> Result<&T, ()>
    where
        T: std::fmt::Debug,
    {
        let mode = CString::new("rb").unwrap();
        unsafe {
            php_sys::ts_resource_ex(0, ptr::null_mut());
            let handle_filename = CString::new(handle_filename).unwrap();
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

pub type StartupCallback<T> = FnMut(&mut T) -> Result<(), ()>;
pub type ShutdownCallback<T> = FnMut(&mut T) -> Result<(), ()>;
pub type WriteCallback<T> = FnMut(&mut T, &[u8]) -> Result<usize, ()>;
pub type ReadCallback<T> = FnMut(&mut T, *mut i8, usize) -> Result<usize, ()>;
struct Callbacks<T> {
    startup: Option<Box<StartupCallback<T>>>,
    shutdown: Option<Box<ShutdownCallback<T>>>,
    write: Option<Box<WriteCallback<T>>>,
    read: Option<Box<ReadCallback<T>>>,
}

/// A simple IOContext that handles reading from a buffer and writing to a buffer.
///
/// This can be used as a demo or example of how to read / write to a context
#[derive(Debug)]
pub struct IOContext {
    /// Output buffer
    pub buffer: Vec<u8>,
    /// Input buffer / body
    pub body: Box<[u8]>,
}

impl IOContext {
    fn write(ctx: &mut IOContext, buf: &[u8]) -> Result<usize, ()> {
        ctx.buffer.extend_from_slice(&buf);
        Ok(buf.len())
    }

    fn read(ctx: &mut IOContext, buf: *mut i8, bytes: usize) -> Result<usize, ()> {
        unsafe {
            let ctx = ctx as *mut IOContext;
            let body = &(*ctx).body;
            let copied = ::std::cmp::min(bytes, body.len());
            if copied > 0 {
                let (to_send, to_retain) = body.split_at(copied);
                let ptr = to_send.as_ptr() as *const i8;
                ::std::ptr::copy(ptr, buf, copied);
                (*ctx).body = to_retain.to_owned().into_boxed_slice();
            }
            Ok(copied)
        }
    }
    /// Adds the IOContext to a builder, this will set all related functions.
    pub fn add_to_builder(builder: RuntimeBuilder<IOContext>) -> RuntimeBuilder<IOContext> {
        builder
            .read(Box::new(IOContext::read))
            .write(Box::new(IOContext::write))
    }
}

/// Runtime builder to set callbacks as required.
pub struct RuntimeBuilder<T> {
    callbacks: Callbacks<T>,
    module: Box<php_sys::_sapi_module_struct>,
}

impl<T> RuntimeBuilder<T> {
    /// The startup callback is called when the php runtime is started. It
    /// can be used to initiate an environment as needed.
    pub fn startup(mut self, callback: Box<StartupCallback<T>>) -> Self {
        self.callbacks.startup = Some(callback);
        self
    }

    /// The shutdown callback is called when the php runtime is terminated. It
    /// can be used to clean up an environment as needed.
    pub fn shutdown(mut self, callback: Box<ShutdownCallback<T>>) -> Self {
        self.callbacks.shutdown = Some(callback);
        self
    }

    /// This is called when the PHP code wants to write data as a return.
    /// The function might be called multiple times per execution!
    pub fn write(mut self, callback: Box<WriteCallback<T>>) -> Self {
        self.callbacks.write = Some(callback);
        self
    }

    /// This is called when the PHP tries to to read the 'body'.
    /// The function might be called multiple times per execution! and
    /// should progressively consume the boy.
    pub fn read(mut self, callback: Box<ReadCallback<T>>) -> Self {
        self.callbacks.read = Some(callback);
        self
    }

    /// Finalizes the builder, creates and starts the runtime.
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_execution() {
        let mut runtime =
            IOContext::add_to_builder(Runtime::new("php-test", "PHP Test Runtime", 1)).start();

        let ctx = IOContext {
            body: "hello".to_string().into_bytes().into_boxed_slice(),
            buffer: Vec::with_capacity(1028),
        };
        let d = ::std::env::current_dir().unwrap();
        let d = d.join("tests/test.php");
        let ctx = runtime.execute(d.to_str().unwrap(), ctx).unwrap();
        assert_eq!(
            String::from_utf8(ctx.buffer.clone()).unwrap(),
            "php got: hello".to_string()
        );
    }
}
