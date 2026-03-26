#[cfg(windows)]
fn main() {
    use winres::WindowsResource;

    let mut res = WindowsResource::new();
    res.set_language(0x0409); // English (US)
    
    // Embed the application manifest for DPI awareness
    res.set_manifest_file("app.manifest");
    
    // Set the main application icon (IDI_MAINICON = 200)
    // This will be used as the exe file icon
    res.set_icon("assets/privoxy.ico");
    
    // Compile the resource file which includes all icons
    res.set_resource_file("privoxy.rc");
    
    res.compile().unwrap();
}

#[cfg(not(windows))]
fn main() {
    // Do nothing for non-Windows platforms
}
