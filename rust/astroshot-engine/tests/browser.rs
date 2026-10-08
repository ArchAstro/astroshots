//! Real-browser tests for `astroshot_engine::browser`. Skipped (with a printed reason)
//! when no Chrome is installed.

mod common;

use std::time::Duration;

use astroshot_engine::browser::{
    Browser, ColorScheme, LaunchOptions, PageOptions, ScreenshotOptions, Size, WaitUntil,
    find_chrome, record_webm, resample_frames,
};

const T: Duration = Duration::from_secs(20);

fn chrome_or_skip() -> bool {
    match find_chrome() {
        Ok(path) => {
            eprintln!("using {}", path.display());
            true
        }
        Err(error) => {
            common::skip(&error);
            false
        }
    }
}

fn data_url(html: &str) -> String {
    let encoded: String = html
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("data:text/html;charset=utf-8,{encoded}")
}

fn decode(png: &[u8]) -> image::RgbaImage {
    image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .unwrap()
        .to_rgba8()
}

const PAGE: &str = r#"<!doctype html><body style="margin:0;background:#fff">
<div id="box" style="position:absolute;left:30px;top:40px;width:120px;height:80px;background:rgb(255,0,0)"></div>
<div id="late" style="display:none">later</div>
<script>setTimeout(() => { document.getElementById('late').style.display='block'; window.READY = true; }, 200);</script>
</body>"#;

#[tokio::test]
async fn element_screenshot_matches_css_size_times_scale_and_color() {
    if !chrome_or_skip() {
        return;
    }
    let browser = Browser::launch(LaunchOptions::default()).await.unwrap();
    let page = browser
        .new_page(PageOptions::new(400, 300).scale(2.0))
        .await
        .unwrap();
    page.goto(&data_url(PAGE), WaitUntil::DomContentLoaded, T)
        .await
        .unwrap();
    page.wait_for_selector("#box", T).await.unwrap();
    let png = page.screenshot_element("#box", false).await.unwrap();
    let img = decode(&png);
    assert_eq!((img.width(), img.height()), (240, 160));
    assert_eq!(img.get_pixel(120, 80).0, [255, 0, 0, 255]);
    assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(img.get_pixel(239, 159).0, [255, 0, 0, 255]);
    let bbox = page.bounding_box("#box").await.unwrap().unwrap();
    assert_eq!(
        (bbox.x, bbox.y, bbox.width, bbox.height),
        (30.0, 40.0, 120.0, 80.0)
    );
    page.close().await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn viewport_scale_full_page_clip_and_color_scheme() {
    if !chrome_or_skip() {
        return;
    }
    let browser = Browser::launch(LaunchOptions::default()).await.unwrap();
    let html = r#"<!doctype html><style>body{margin:0;height:900px;background:#fff}
        @media (prefers-color-scheme: dark){ body{background:#101010} }</style><body>"#;
    let page = browser
        .new_page(
            PageOptions::new(200, 100)
                .scale(1.5)
                .color_scheme(ColorScheme::Dark),
        )
        .await
        .unwrap();
    page.goto(&data_url(html), WaitUntil::Load, T)
        .await
        .unwrap();
    assert_eq!(
        page.viewport(),
        Size {
            width: 200,
            height: 100
        }
    );

    // Viewport shot: 200x100 CSS px at 1.5x, dark scheme applied.
    let img = decode(&page.screenshot(ScreenshotOptions::default()).await.unwrap());
    assert_eq!((img.width(), img.height()), (300, 150));
    assert_eq!(img.get_pixel(10, 10).0, [16, 16, 16, 255]);

    // Full page covers the 900px document.
    let full = decode(
        &page
            .screenshot(ScreenshotOptions {
                full_page: true,
                ..Default::default()
            })
            .await
            .unwrap(),
    );
    assert_eq!((full.width(), full.height()), (300, 1350));

    // Clip is in CSS pixels.
    let clip = decode(
        &page
            .screenshot(ScreenshotOptions {
                clip: Some(astroshot_engine::browser::BoundingBox {
                    x: 0.0,
                    y: 0.0,
                    width: 50.0,
                    height: 20.0,
                }),
                ..Default::default()
            })
            .await
            .unwrap(),
    );
    assert_eq!((clip.width(), clip.height()), (75, 30));

    // Resizing keeps the scale; omit_background yields transparency.
    page.set_viewport(100, 60).await.unwrap();
    let small = decode(&page.screenshot(ScreenshotOptions::default()).await.unwrap());
    assert_eq!((small.width(), small.height()), (150, 90));
    page.evaluate::<bool>("(document.body.style.background = 'transparent', document.documentElement.style.background = 'transparent', true)")
        .await
        .unwrap();
    let transparent = decode(
        &page
            .screenshot(ScreenshotOptions {
                omit_background: true,
                ..Default::default()
            })
            .await
            .unwrap(),
    );
    assert_eq!(transparent.get_pixel(5, 5).0[3], 0);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn waits_errors_and_set_content() {
    if !chrome_or_skip() {
        return;
    }
    let browser = Browser::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page(PageOptions::new(300, 200)).await.unwrap();
    page.goto(&data_url(PAGE), WaitUntil::DomContentLoaded, T)
        .await
        .unwrap();

    page.wait_for_function("window.READY === true", T)
        .await
        .unwrap();
    page.wait_for_selector("#late", T).await.unwrap();
    page.wait_for_text("LATER", T).await.unwrap();
    assert_eq!(page.inner_text("#late").await.unwrap(), "later");
    assert_eq!(page.count("div").await.unwrap(), 2);
    assert_eq!(page.evaluate::<i64>("1 + 2").await.unwrap(), 3);

    let missing = page
        .wait_for_selector("#nope", Duration::from_millis(150))
        .await;
    assert!(missing.unwrap_err().to_string().contains("Timed out"));
    assert!(page.screenshot_element("#nope", false).await.is_err());
    assert!(
        page.evaluate::<i64>("throw new Error('boom')")
            .await
            .is_err()
    );

    page.evaluate::<bool>("(console.error('visible-console-error'), setTimeout(() => { throw new Error('async-boom') }, 0), true)")
        .await
        .unwrap();
    page.wait_for_timeout(300).await;
    let errors = page.page_errors();
    assert!(
        errors.iter().any(|e| e == "visible-console-error"),
        "{errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|e| e.starts_with("Error: async-boom") && !e.contains("\n")),
        "{errors:?}"
    );

    page.set_content("<body><p id=p>hi</p></body>")
        .await
        .unwrap();
    assert_eq!(page.inner_text("#p").await.unwrap(), "hi");

    let bad = page.goto("http://127.0.0.1:1/", WaitUntil::Load, T).await;
    assert!(bad.is_err());
    browser.close().await.unwrap();
}

#[tokio::test]
async fn concurrent_pages_render_independently() {
    if !chrome_or_skip() {
        return;
    }
    let browser = Browser::launch(LaunchOptions::default()).await.unwrap();
    let mut jobs = Vec::new();
    for i in 0..6u32 {
        let browser = browser.clone();
        jobs.push(tokio::spawn(async move {
            let width = 40 + i * 10;
            let page = browser.new_page(PageOptions::new(200, 200).scale(1.0 + f64::from(i % 2))).await.unwrap();
            let color = i * 40;
            let html = format!(
                "<!doctype html><body style='margin:0'><div id=b style='width:{width}px;height:30px;background:rgb({color},0,0)'></div>"
            );
            page.goto(&data_url(&html), WaitUntil::Load, T).await.unwrap();
            let png = page.screenshot_element("#b", false).await.unwrap();
            page.close().await.unwrap();
            (i, width, decode(&png))
        }));
    }
    for job in jobs {
        let (i, width, img) = job.await.unwrap();
        let scale = 1 + i % 2;
        assert_eq!((img.width(), img.height()), (width * scale, 30 * scale));
        assert_eq!(img.get_pixel(1, 1).0, [(i * 40) as u8, 0, 0, 255]);
    }
    browser.close().await.unwrap();
}

#[tokio::test]
async fn shared_browser_is_reused_then_closed() {
    if !chrome_or_skip() {
        return;
    }
    let a = Browser::shared(false).await.unwrap();
    let b = Browser::shared(false).await.unwrap();
    let page = a.new_page(PageOptions::new(50, 50)).await.unwrap();
    let from_b = b.new_page(PageOptions::new(50, 50)).await.unwrap();
    assert!(a.is_connected() && b.is_connected());
    page.close().await.unwrap();
    from_b.close().await.unwrap();
    Browser::close_shared().await.unwrap();
    assert!(!a.is_connected());
    // Next call relaunches.
    let c = Browser::shared(false).await.unwrap();
    assert!(c.is_connected());
    Browser::close_shared().await.unwrap();
}

#[tokio::test]
async fn screencast_frames_resample_to_constant_fps() {
    if !chrome_or_skip() {
        return;
    }
    let browser = Browser::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page(PageOptions::new(160, 90)).await.unwrap();
    page.set_content("<body style='margin:0;background:#000'><div id=d style='width:50px;height:50px;background:#0f0'></div></body>")
        .await
        .unwrap();
    page.start_screencast(None).await.unwrap();
    for i in 0..8 {
        page.evaluate::<bool>(&format!(
            "(document.getElementById('d').style.marginLeft = '{}px', true)",
            i * 10
        ))
        .await
        .unwrap();
        page.wait_for_timeout(60).await;
    }
    let frames = page.stop_screencast().await.unwrap();
    assert!(!frames.is_empty());
    let sampled = resample_frames(&frames, 10, 500);
    assert_eq!(sampled.len(), 5);
    assert!(sampled[0].starts_with(&[0x89, b'P', b'N', b'G']));
    page.close().await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn record_webm_replays_png_frames() {
    if !chrome_or_skip() {
        return;
    }
    let browser = Browser::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page(PageOptions::new(64, 48)).await.unwrap();
    page.set_content(
        "<body style='margin:0;background:#00f'><div style='width:64px;height:48px'></div></body>",
    )
    .await
    .unwrap();
    let frame = page.screenshot(ScreenshotOptions::default()).await.unwrap();
    page.close().await.unwrap();
    let frames = vec![frame.clone(), frame.clone(), frame];
    let webm = record_webm(
        &browser,
        &frames,
        Size {
            width: 64,
            height: 48,
        },
        10,
    )
    .await
    .unwrap();
    // EBML header magic.
    assert_eq!(&webm[..4], &[0x1A, 0x45, 0xDF, 0xA3]);
    browser.close().await.unwrap();
}
