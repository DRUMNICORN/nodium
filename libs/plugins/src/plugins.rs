use crate::plugin_utils::{download, install};
use crate::Registry;
use dirs_next::document_dir;
use libloading::{Library, Symbol};
use log::{debug, error, info, warn};
use nodium_events::{NodiumEventBus, NodiumEventType};
use nodium_pdk::NodiumPlugin;
use serde_json::Value;
use std::fmt::Debug;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct NodiumPlugins {
    install_location: String,
    registry: Registry,
}

impl Debug for NodiumPlugins {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodiumPlugins")
            .field("install_location", &self.install_location)
            .finish()
    }
}

impl NodiumPlugins {
    pub fn new() -> Arc<Mutex<Self>> {
        let doc_dir = document_dir().expect("Unable to get user's document directory");
        let install_location = doc_dir.join("nodium").join("plugins");
        debug!("Plugin install location: {:?}", install_location);
        if !install_location.exists() {
            debug!("Creating plugin directory");
            fs::create_dir_all(&install_location).expect("Unable to create plugin directory");
        }
        let plugins = NodiumPlugins {
            install_location: "plugins".to_string(),
            registry: Registry::new(),
        };

        let installer = Arc::new(Mutex::new(plugins));
        let installer_clone = installer.clone();
        tokio::spawn(async move {
          installer_clone.lock().await.listen(installer_clone.clone()).await;
        });
        installer
    }

    pub async fn reload(&mut self) {
        debug!("Reloading plugins");
        let plugins_dir = Path::new(&self.install_location);
        if !plugins_dir.exists() {
            debug!("Plugins directory does not exist");
            match fs::create_dir_all(&plugins_dir) {
                Ok(_) => {
                    debug!("Plugins directory created successfully");
                }
                Err(e) => {
                    error!("Error creating plugins directory: {}", e);
                    return;
                }
            }
        }
        let folders = match fs::read_dir(&plugins_dir) {
            Ok(folders) => folders,
            Err(e) => {
                error!("Error reading plugins directory: {}", e);
                return;
            }
        };

        for entry in folders {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    error!("Error reading plugin directory: {}", e);
                    continue;
                }
            };
            let path = entry.path();
            debug!("Plugin path: {:?}", path);
            if path.is_dir() {
                let plugin_name = path.file_name().unwrap().to_str().unwrap();
                let plugin_version = path.file_name().unwrap().to_str().unwrap();
                debug!(
                    "Plugin name and version: {} {}",
                    plugin_name, plugin_version
                );
                match self.register(plugin_name, plugin_version, true) {
                    Ok(_) => {
                        info!("Plugin registered successfully");
                    }
                    Err(e) => {
                        warn!("Plugin not able to register: {}", e);
                        // get folder name of last folder in path
                        match install(
                            path.file_name().unwrap().to_str().unwrap(),
                            "",
                            &self.install_location,
                            true,
                        )
                        .await
                        {
                            Ok(_) => {
                              match self.register(plugin_name, plugin_version, true) {
                                Ok(_) => {
                                    info!("Plugin registered successfully");
                                }
                                Err(e) => {
                                    error!("Error registering plugin: {}", e);
                                }
                              }
                            }
                            Err(e) => {
                                error!("Error installing plugin: {}", e);
                            }
                        }
                    }
                }
            }
        }
    }

    pub async fn listen(&self, plugins: Arc<Mutex<Self>>) {
        let plugins_clone = plugins.clone();
        let plugins_clone_callback = plugins_clone.clone();


        // TODO: load plugins in the plugins directory
        // events
        //     .register(
        //         &NodiumEventType::PluginInstall.to_string(),
        //         Box::new(move |payload| {
        //             let payload: Value = serde_json::from_str(&payload).unwrap();
        //             debug!("Plugin install payload: {:?}", payload);

        //             let plugins = plugins_clone_callback.clone();
        //             debug!("Plugins: {:?}", plugins);

        //             // tokio::spawn(async move {
        //             //     match plugins
        //             //         .lock()
        //             //         .await
        //             //         .register(plugin_name, plugin_version, false)
        //             //     {
        //             //         Ok(_) => {
        //             //             info!("Plugin registered successfully");
        //             //         }
        //             //         Err(e) => {
        //             //             error!("Error registering plugin: {}", e);
        //             //         }
        //             //     }
        //             // });
        //         }),
        //     )
        //     .await;
        // events
        //     .register(
        //         &NodiumEventType::PluginsReload.to_string(),
        //         Box::new(move |_| {
        //             let plugins = plugins_clone.clone();
        //             tokio::spawn(async move {
        //                 plugins.lock().await.reload().await;
        //             });
        //         }),
        //     )
        //     .await;
    }

    // TODO: add plugin version
    async fn plugin_install(
        &mut self,
        payload: String,
        install_location: String,
    ) -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
        let install_location = install_location.clone();
        debug!("Installing crate {}", payload);
        let data: Value = serde_json::from_str(&payload).unwrap();
        debug!("Handling event: {}", payload);
        debug!("Event data: {:?}", data);

        let crate_version = data
            .get("crate_version")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        let crate_name = data
            .get("crate_name")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();

        match download(&crate_name, &crate_version, &install_location).await {
            Ok(_) => {
                info!("Crate downloaded successfully");
                match install(&crate_name, &crate_version, &install_location, false).await {
                    Ok(_) => {
                        info!("Crate installed successfully");
                        Ok((crate_name, crate_version))
                    }
                    Err(e) => {
                        error!("Error installing crate: {}", e);
                        Err(e)
                    }
                }
            }
            Err(e) => {
                error!("Error downloading crate: {}", e);
                Err(e)
            }
        }
    }

    fn register(
        &mut self,
        crate_name: &str,
        crate_version: &str,
        is_local: bool,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let folder_name = if is_local {
            crate_name.to_string()
        } else {
            format!("{}-{}", crate_name, crate_version)
        };
        let lib_path = Path::new(&self.install_location)
            .join(folder_name)
            .join("target")
            .join("release")
            .join(if cfg!(windows) { "lib" } else { "" })
            .join(format!(
                "lib{}{}",
                crate_name,
                if cfg!(windows) {
                    ".dll"
                } else if cfg!(unix) {
                    ".so"
                } else { // Todo: add support for other platforms (macos, ios, android, etc.)
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Unsupported platform",
                    )));
                }
            ));
        
        // TODO: Check out: abi_stable and Foreign Function Interface

        let lib = unsafe { Library::new(&lib_path) }?;
        let plugin: Symbol<unsafe extern "C" fn() -> Box<dyn NodiumPlugin>> =
            unsafe { lib.get(b"plugin")? };

        let plugin = unsafe { plugin() };
        let plugin_name = plugin.name();
        debug!("Registering plugin: {}", plugin_name);
        self.registry.register_plugin(plugin);
        Ok(())
    }

    pub fn get_plugins(&self) -> Vec<String> {
        debug!("Getting plugins");
        let plugins = self.registry.get_plugins();
        debug!("Plugins: {:?}", plugins);
        plugins
    }
}