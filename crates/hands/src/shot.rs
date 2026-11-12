//! `screenshot`: the screen you're on, saved to Pictures › Screenshots, without her in it.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::tool::{Ctx, Outcome, Risk, Tool, ToolSpec, opt_arg};

pub struct Screenshot;

#[async_trait]
impl Tool for Screenshot {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "screenshot".into(),
            description: "Takes a screenshot of the screen the window in front is on (she isn't in it) and saves it to \
                          Pictures › Screenshots. `open`: open it after."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "open": { "type": "boolean" } }
            }),
        }
    }

    fn risk(&self, _: &Value) -> Risk {
        Risk::Normal
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        if ctx.dry_run {
            return Outcome::ok("(dry run) would take a screenshot");
        }
        let Some(dir) = directories::UserDirs::new().and_then(|d| d.picture_dir().map(|p| p.join("Screenshots"))) else {
            return Outcome::failed("couldn't find the Pictures folder");
        };
        let shot = match tokio::task::spawn_blocking(crate::platform::screen_shot).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Outcome::failed(format!("couldn't take it: {e:#}")),
            Err(e) => return Outcome::failed(format!("couldn't take it: {e}")),
        };
        let name = chrono::Local::now().format("Capri %Y-%m-%d %H-%M-%S.png").to_string();
        let path = dir.join(&name);
        let saved = std::fs::create_dir_all(&dir).map_err(anyhow::Error::from).and_then(|_| {
            let file = std::io::BufWriter::new(std::fs::File::create(&path)?);
            let mut enc = png::Encoder::new(file, shot.width, shot.height);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header()?.write_image_data(&shot.rgb)?;
            Ok(())
        });
        if let Err(e) = saved {
            return Outcome::failed(format!("couldn't save it: {e:#}"));
        }
        let shown = path.display().to_string();
        if opt_arg(&args, "open").unwrap_or(false) {
            let _ = crate::pc::shell_open(&shown);
        }
        Outcome::ok(format!("saved to Pictures › Screenshots › {name} ({}×{})", shot.width, shot.height)).with(json!({ "path": shown }))
    }
}
