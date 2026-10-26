// Embeds the app icon into the Windows executable. Other platforms do nothing.
fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("packaging/icons/icon.ico");
        res.set("ProductName", "Ferrum");
        res.set("FileDescription", "Ferrum SSH client");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the Windows icon: {e}");
        }
    }
}
