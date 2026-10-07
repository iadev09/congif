use congif::{Configurable, Result, async_trait};

struct HttpProvider;
struct HTTPProvider;
struct GenericProvider<T>(T);
struct NamedProvider;

macro_rules! configuration_owner {
    ($owner:ty) => {
        #[async_trait]
        impl Configurable for $owner {
            type Config = ();

            fn source(&self) -> congif::ConfigFile {
                congif::ConfigFile {
                    path: "example.json".into(),
                    format: congif::Format::Json,
                }
            }

            async fn configure(
                &self,
                _config: &(),
            ) -> Result<()> {
                Ok(())
            }
        }
    };
}

configuration_owner!(HttpProvider);
configuration_owner!(HTTPProvider);
configuration_owner!(GenericProvider<HttpProvider>);

#[async_trait]
impl Configurable for NamedProvider {
    type Config = ();

    fn source(&self) -> congif::ConfigFile {
        congif::ConfigFile {
            path: "example.json".into(),
            format: congif::Format::Json,
        }
    }

    fn namespace(&self) -> String {
        "http".into()
    }

    async fn configure(
        &self,
        _config: &(),
    ) -> Result<()> {
        Ok(())
    }
}

#[test]
fn default_namespace_uses_the_owner_name_without_modules_or_generic_arguments()
{
    assert_eq!(HttpProvider.namespace(), "http_provider");
    assert_eq!(HTTPProvider.namespace(), "http_provider");
    assert_eq!(GenericProvider(HttpProvider).namespace(), "generic_provider");
}

#[test]
fn owner_can_choose_a_public_namespace() {
    assert_eq!(NamedProvider.namespace(), "http");
}
