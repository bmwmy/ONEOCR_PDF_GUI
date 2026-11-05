// src/ocr_ffi.rs
//! Direct Win32 FFI and Dynamic Loader for oneocr.dll (Microsoft Windows Snipping Tool OCR Engine)

use libloading::Library;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// Microsoft Snipping Tool OneOCR decryption key for oneocr.onemodel
const MODEL_KEY: &[u8] = b"kj)TGtrK>f]b[Piow.gU+nC@s\"\"\"\"\"\"4\0";

/// OneOCR raw image structure required by RunOcrPipeline
#[repr(C)]
pub struct ImageStructure {
    pub img_type: i32,      // 1 for standard RGB/BGR buffer
    pub width: i32,         // Width in pixels
    pub height: i32,        // Height in pixels
    pub reserved: i32,      // 0
    pub step_size: i64,     // Bytes per row (width * 4)
    pub data_ptr: *const u8,// Pointer to raw pixel buffer
}

/// OneOCR bounding quadrilateral
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct OcrRect {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub x3: f32,
    pub y3: f32,
    pub x4: f32,
    pub y4: f32,
}

#[derive(Debug, Clone)]
pub struct OcrWord {
    pub text: String,
    pub rect: OcrRect,
    pub confidence: f32,
}

#[derive(Debug, Clone)]
pub struct OcrLine {
    pub words: Vec<OcrWord>,
}

#[derive(Debug, Clone)]
pub struct OcrResult {
    pub lines: Vec<OcrLine>,
}

// Function pointer signatures for oneocr.dll exported C functions
type FnCreateOcrInitOptions = unsafe extern "C" fn(out_options: *mut i64) -> i64;
type FnOcrInitOptionsSetUseModelDelayLoad = unsafe extern "C" fn(options: i64, flag: u8) -> i64;
type FnCreateOcrPipeline = unsafe extern "C" fn(
    model_path: *const c_char,
    model_key: *const c_char,
    init_options: i64,
    out_pipeline: *mut i64,
) -> i64;
type FnCreateOcrProcessOptions = unsafe extern "C" fn(out_options: *mut i64) -> i64;
type FnRunOcrPipeline = unsafe extern "C" fn(
    pipeline: i64,
    image_struct: *const ImageStructure,
    process_options: i64,
    out_result: *mut i64,
) -> i64;
type FnGetOcrLineCount = unsafe extern "C" fn(result: i64, out_count: *mut i64) -> i64;
type FnGetOcrLine = unsafe extern "C" fn(result: i64, index: i64, out_line: *mut i64) -> i64;
type FnGetOcrLineWordCount = unsafe extern "C" fn(line: i64, out_count: *mut i64) -> i64;
type FnGetOcrWord = unsafe extern "C" fn(line: i64, index: i64, out_word: *mut i64) -> i64;
type FnGetOcrWordContent = unsafe extern "C" fn(word: i64, out_content: *mut *const c_char) -> i64;
type FnGetOcrWordBoundingBox = unsafe extern "C" fn(word: i64, out_box: *mut *const OcrRect) -> i64;

pub struct OneOcrEngine {
    _lib: Arc<Library>,
    pipeline_handle: i64,
    process_options_handle: i64,
    run_pipeline_fn: FnRunOcrPipeline,
    get_line_count_fn: FnGetOcrLineCount,
    get_line_fn: FnGetOcrLine,
    get_word_count_fn: FnGetOcrLineWordCount,
    get_word_fn: FnGetOcrWord,
    get_word_content_fn: FnGetOcrWordContent,
    get_word_box_fn: FnGetOcrWordBoundingBox,
}

unsafe impl Send for OneOcrEngine {}
unsafe impl Sync for OneOcrEngine {}

impl OneOcrEngine {
    /// Discovers oneocr.dll beside built binary first, then in ~/.config/oneocr or APPDATA
    pub fn new() -> Result<Self, String> {
        Self::ensure_oneocr_files();

        let dll_path = Self::find_oneocr_dll()?;
        let model_path = Self::find_oneocr_model()?;

        // Instruct Windows loader to search in the folder of oneocr.dll so it locates onnxruntime.dll
        if let Some(parent_dir) = dll_path.parent() {
            Self::set_dll_directory(parent_dir);
        }

        unsafe {
            let lib = Library::new(&dll_path)
                .map_err(|e| format!("Failed to load oneocr.dll from {:?}: {}", dll_path, e))?;

            let create_init_options: FnCreateOcrInitOptions = *lib.get(b"CreateOcrInitOptions")
                .map_err(|e| format!("Missing CreateOcrInitOptions: {}", e))?;
            let set_delay_load: FnOcrInitOptionsSetUseModelDelayLoad = *lib.get(b"OcrInitOptionsSetUseModelDelayLoad")
                .map_err(|e| format!("Missing OcrInitOptionsSetUseModelDelayLoad: {}", e))?;
            let create_pipeline: FnCreateOcrPipeline = *lib.get(b"CreateOcrPipeline")
                .map_err(|e| format!("Missing CreateOcrPipeline: {}", e))?;
            let create_process_options: FnCreateOcrProcessOptions = *lib.get(b"CreateOcrProcessOptions")
                .map_err(|e| format!("Missing CreateOcrProcessOptions: {}", e))?;
            let run_pipeline_fn: FnRunOcrPipeline = *lib.get(b"RunOcrPipeline")
                .map_err(|e| format!("Missing RunOcrPipeline: {}", e))?;
            let get_line_count_fn: FnGetOcrLineCount = *lib.get(b"GetOcrLineCount")
                .map_err(|e| format!("Missing GetOcrLineCount: {}", e))?;
            let get_line_fn: FnGetOcrLine = *lib.get(b"GetOcrLine")
                .map_err(|e| format!("Missing GetOcrLine: {}", e))?;
            let get_word_count_fn: FnGetOcrLineWordCount = *lib.get(b"GetOcrLineWordCount")
                .map_err(|e| format!("Missing GetOcrLineWordCount: {}", e))?;
            let get_word_fn: FnGetOcrWord = *lib.get(b"GetOcrWord")
                .map_err(|e| format!("Missing GetOcrWord: {}", e))?;
            let get_word_content_fn: FnGetOcrWordContent = *lib.get(b"GetOcrWordContent")
                .map_err(|e| format!("Missing GetOcrWordContent: {}", e))?;
            let get_word_box_fn: FnGetOcrWordBoundingBox = *lib.get(b"GetOcrWordBoundingBox")
                .map_err(|e| format!("Missing GetOcrWordBoundingBox: {}", e))?;

            // 1. Initialize Options
            let mut init_options: i64 = 0;
            create_init_options(&mut init_options);
            set_delay_load(init_options, 0);

            // 2. Prepare model path C-string
            let model_c_str = CString::new(model_path.to_string_lossy().as_bytes())
                .map_err(|e| format!("Invalid model path: {}", e))?;

            // 3. Create Pipeline using the model decryption key
            let mut pipeline_handle: i64 = 0;
            let create_status = create_pipeline(
                model_c_str.as_ptr(),
                MODEL_KEY.as_ptr() as *const c_char,
                init_options,
                &mut pipeline_handle,
            );

            if create_status != 0 || pipeline_handle == 0 {
                return Err(format!("CreateOcrPipeline failed with error code: {}", create_status));
            }

            // 4. Create Process Options
            let mut process_options: i64 = 0;
            create_process_options(&mut process_options);

            Ok(Self {
                _lib: Arc::new(lib),
                pipeline_handle,
                process_options_handle: process_options,
                run_pipeline_fn,
                get_line_count_fn,
                get_line_fn,
                get_word_count_fn,
                get_word_fn,
                get_word_content_fn,
                get_word_box_fn,
            })
        }
    }

    /// Performs OCR recognition on image buffer (converts RGBA to BGRA with img_type=3 required by oneocr.dll)
    pub fn recognize(&self, image: &image::RgbaImage) -> Result<OcrResult, String> {
        let (width, height) = image.dimensions();
        // Microsoft OneOCR natively requires 4-channel BGRA byte format with img_type = 3.
        // Previously passing img_type = 1 with a 4-byte row stride caused OneOCR to read pixels as 3-byte BGR,
        // misinterpreting the stride and truncating the row by 25% (4/3 factor), which cut off the entire right margin!
        let mut bgra_bytes = Vec::with_capacity((width * height * 4) as usize);
        for pixel in image.pixels() {
            let [r, g, b, a] = pixel.0;
            bgra_bytes.push(b);
            bgra_bytes.push(g);
            bgra_bytes.push(r);
            bgra_bytes.push(a);
        }

        let img_struct = ImageStructure {
            img_type: 3, // 3 for 4-channel BGRA format
            width: width as i32,
            height: height as i32,
            reserved: 0,
            step_size: (width * 4) as i64,
            data_ptr: bgra_bytes.as_ptr(),
        };

        let mut result_handle: i64 = 0;
        let status = unsafe {
            (self.run_pipeline_fn)(
                self.pipeline_handle,
                &img_struct,
                self.process_options_handle,
                &mut result_handle,
            )
        };

        if status != 0 || result_handle == 0 {
            return Err(format!("RunOcrPipeline execution failed with code {}", status));
        }

        // Query lines count
        let mut line_count: i64 = 0;
        unsafe {
            (self.get_line_count_fn)(result_handle, &mut line_count);
        }

        let mut lines = Vec::new();
        for l_idx in 0..line_count {
            let mut line_handle: i64 = 0;
            unsafe {
                (self.get_line_fn)(result_handle, l_idx, &mut line_handle);
            }

            let mut word_count: i64 = 0;
            unsafe {
                (self.get_word_count_fn)(line_handle, &mut word_count);
            }

            let mut words = Vec::new();
            for w_idx in 0..word_count {
                let mut word_handle: i64 = 0;
                unsafe {
                    (self.get_word_fn)(line_handle, w_idx, &mut word_handle);
                }

                let mut c_str_ptr: *const c_char = std::ptr::null();
                unsafe {
                    (self.get_word_content_fn)(word_handle, &mut c_str_ptr);
                }

                let text = if !c_str_ptr.is_null() {
                    unsafe { CStr::from_ptr(c_str_ptr).to_string_lossy().into_owned() }
                } else {
                    String::new()
                };

                let mut box_ptr: *const OcrRect = std::ptr::null();
                unsafe {
                    (self.get_word_box_fn)(word_handle, &mut box_ptr);
                }

                let rect = if !box_ptr.is_null() {
                    unsafe { *box_ptr }
                } else {
                    OcrRect::default()
                };

                words.push(OcrWord {
                    text,
                    rect,
                    confidence: 1.0,
                });
            }

            lines.push(OcrLine { words });
        }

        Ok(OcrResult { lines })
    }

    /// Instructs the Windows dynamic loader where to resolve dependency DLLs (e.g. onnxruntime.dll)
    fn set_dll_directory(dir: &Path) {
        #[cfg(windows)]
        unsafe {
            use std::os::windows::ffi::OsStrExt;
            let mut wide_dir: Vec<u16> = dir.as_os_str().encode_wide().collect();
            wide_dir.push(0);
            
            // Call Win32 SetDllDirectoryW from kernel32
            #[link(name = "kernel32")]
            extern "system" {
                fn SetDllDirectoryW(lpPathName: *const u16) -> i32;
            }
            SetDllDirectoryW(wide_dir.as_ptr());
        }
    }

    /// Automatically checks if model files exist in ~/.config/oneocr and copies them from beside the executable if found
    pub fn ensure_oneocr_files() {
        let user_config_dir = dirs::config_dir().map(|d| d.join("oneocr"));
        if let Some(target_dir) = user_config_dir {
            let _ = std::fs::create_dir_all(&target_dir);

            if let Ok(exe_path) = std::env::current_exe() {
                if let Some(exe_dir) = exe_path.parent() {
                    let required_files = ["oneocr.dll", "oneocr.onemodel", "onnxruntime.dll", "pdfium.dll"];
                    for f in &required_files {
                        let target_file = target_dir.join(f);
                        let source_file = exe_dir.join(f);
                        if !target_file.exists() && source_file.exists() {
                            let _ = std::fs::copy(&source_file, &target_file);
                        }
                    }
                }
            }
        }
    }

    /// Finds oneocr.dll: checks beside the built binary FIRST, then current dir, then ~/.config/oneocr
    pub fn find_oneocr_dll() -> Result<PathBuf, String> {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf()));
        let candidate_dirs = [
            exe_dir,                                              // 1. Priority: Beside the built binary
            std::env::current_dir().ok(),                          // 2. Priority: Current working directory
            dirs::config_dir().map(|d| d.join("oneocr")),         // 3. Priority: ~/.config/oneocr/
            dirs::home_dir().map(|d| d.join(".config").join("oneocr")), // 4. %USERPROFILE%/.config/oneocr
        ];

        for dir in candidate_dirs.into_iter().flatten() {
            let p = dir.join("oneocr.dll");
            if p.exists() {
                return Ok(p);
            }
        }
        Err("Could not find oneocr.dll beside built executable, in current folder, or ~/.config/oneocr".into())
    }

    /// Finds oneocr.onemodel: checks beside the built binary FIRST, then current dir, then ~/.config/oneocr
    pub fn find_oneocr_model() -> Result<PathBuf, String> {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf()));
        let candidate_dirs = [
            exe_dir,                                              // 1. Priority: Beside the built binary
            std::env::current_dir().ok(),                          // 2. Priority: Current working directory
            dirs::config_dir().map(|d| d.join("oneocr")),         // 3. Priority: ~/.config/oneocr/
            dirs::home_dir().map(|d| d.join(".config").join("oneocr")), // 4. %USERPROFILE%/.config/oneocr
        ];

        for dir in candidate_dirs.into_iter().flatten() {
            let p = dir.join("oneocr.onemodel");
            if p.exists() {
                return Ok(p);
            }
        }
        Err("Could not find oneocr.onemodel beside built executable, in current folder, or ~/.config/oneocr".into())
    }
}
