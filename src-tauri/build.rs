fn main() {
  // oneocr-rs khai báo `oneocr.dll` bằng import TĨNH (windows_link::link! + raw-dylib).
  // Thiếu DLL thì process chết ngay lúc nạp (STATUS_DLL_NOT_FOUND 0xc0000135) — chưa kịp chạy
  // code chép file. Ép delay-load để chỉ resolve khi gọi hàm đầu tiên, tức là SAU khi
  // oneocr::ensure_runtime() đã chép đủ 3 file vào cạnh exe.
  println!("cargo:rustc-link-arg=/DELAYLOAD:oneocr.dll");
  println!("cargo:rustc-link-arg=delayimp.lib");

  tauri_build::build()
}
