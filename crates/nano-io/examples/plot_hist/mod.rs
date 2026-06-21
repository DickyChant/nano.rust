use std::error::Error;
use std::path::Path;

use kuva::backend::svg::SvgBackend;
use kuva::plot::Histogram;
use kuva::render::layout::Layout;
use kuva::render::plots::Plot;
use kuva::render::render::render_multiple;
#[cfg(feature = "plot-png")]
use kuva::PngBackend;

#[derive(Debug, Clone, Copy)]
pub struct HistogramSpec<'a> {
    pub title: &'a str,
    pub x_label: &'a str,
    pub y_label: &'a str,
    pub bins: usize,
    pub range: (f64, f64),
    pub color: &'a str,
}

pub fn write_histogram(
    path: &str,
    values: &[f64],
    spec: HistogramSpec<'_>,
) -> Result<(), Box<dyn Error>> {
    let scene = histogram_scene(values, spec)?;
    match output_kind(path) {
        PlotOutput::Svg => {
            let svg = SvgBackend.render_scene(&scene);
            std::fs::write(path, svg)?;
        }
        PlotOutput::Png => write_png(path, &scene)?,
    }
    Ok(())
}

#[cfg(test)]
fn render_histogram_svg(values: &[f64], spec: HistogramSpec<'_>) -> Result<String, Box<dyn Error>> {
    let scene = histogram_scene(values, spec)?;
    Ok(SvgBackend.render_scene(&scene))
}

fn histogram_scene(
    values: &[f64],
    spec: HistogramSpec<'_>,
) -> Result<kuva::render::render::Scene, Box<dyn Error>> {
    if spec.bins == 0 {
        return Err("plot histogram requires at least one bin".into());
    }
    if !spec.range.0.is_finite() || !spec.range.1.is_finite() || spec.range.0 >= spec.range.1 {
        return Err("plot histogram range must be finite and increasing".into());
    }

    let histogram = Histogram::new()
        .with_data(values.iter().copied())
        .with_bins(spec.bins)
        .with_range(spec.range)
        .with_color(spec.color);
    let plots = vec![Plot::Histogram(histogram)];
    let layout = Layout::auto_from_plots(&plots)
        .with_title(spec.title)
        .with_x_label(spec.x_label)
        .with_y_label(spec.y_label)
        .with_width(900.0)
        .with_height(600.0);
    Ok(render_multiple(plots, layout))
}

#[cfg(feature = "plot-png")]
fn write_png(path: &str, scene: &kuva::render::render::Scene) -> Result<(), Box<dyn Error>> {
    let png = PngBackend::new()
        .render_scene(scene)
        .map_err(|err| format!("failed to render PNG plot: {err}"))?;
    std::fs::write(path, png)?;
    Ok(())
}

#[cfg(not(feature = "plot-png"))]
fn write_png(_path: &str, _scene: &kuva::render::render::Scene) -> Result<(), Box<dyn Error>> {
    Err("PNG plot output requires the nano-io `plot-png` feature; rebuild with --features \"plot plot-png\"".into())
}

#[derive(Debug, Clone, Copy)]
enum PlotOutput {
    Svg,
    Png,
}

fn output_kind(path: &str) -> PlotOutput {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some(extension) if extension.eq_ignore_ascii_case("png") => PlotOutput::Png,
        _ => PlotOutput::Svg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_non_empty_svg() {
        let svg = render_histogram_svg(
            &[1.0, 1.5, 2.0, 2.5, 3.0],
            HistogramSpec {
                title: "Test histogram",
                x_label: "x",
                y_label: "count",
                bins: 3,
                range: (1.0, 4.0),
                color: "steelblue",
            },
        )
        .expect("histogram SVG renders");

        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
    }
}
