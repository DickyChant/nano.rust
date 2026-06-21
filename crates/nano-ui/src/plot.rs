pub fn ascii_histogram(values: &[f64], bins: usize, range: Option<(f64, f64)>) -> String {
    if values.is_empty() {
        return "no selected values".to_string();
    }

    let bins = bins.max(1);
    let (mut low, mut high) = range.unwrap_or_else(|| value_range(values));
    if !low.is_finite() || !high.is_finite() || low >= high {
        low = 0.0;
        high = 1.0;
    }

    let width = (high - low) / bins as f64;
    let mut counts = vec![0_usize; bins];
    for value in values {
        if !value.is_finite() || *value < low || *value > high {
            continue;
        }
        let index = if *value == high {
            bins - 1
        } else {
            ((*value - low) / width).floor() as usize
        };
        if let Some(count) = counts.get_mut(index) {
            *count += 1;
        }
    }

    let max_count = counts.iter().copied().max().unwrap_or(0).max(1);
    counts
        .iter()
        .enumerate()
        .map(|(index, count)| {
            let start = low + index as f64 * width;
            let stop = start + width;
            let bar_len = ((*count as f64 / max_count as f64) * 40.0).round() as usize;
            format!("{start:8.2}-{stop:<8.2} {count:5} {}", "#".repeat(bar_len))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn value_range(values: &[f64]) -> (f64, f64) {
    let low = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .fold(f64::INFINITY, f64::min);
    let high = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .fold(f64::NEG_INFINITY, f64::max);
    if low == high {
        (low - 0.5, high + 0.5)
    } else {
        (low, high)
    }
}

#[cfg(feature = "web")]
pub fn histogram_svg(values: &[f64]) -> std::result::Result<String, Box<dyn std::error::Error>> {
    use kuva::backend::svg::SvgBackend;
    use kuva::plot::Histogram;
    use kuva::render::layout::Layout;
    use kuva::render::plots::Plot;
    use kuva::render::render::render_multiple;

    let range = if values.is_empty() {
        (0.0, 1.0)
    } else {
        value_range(values)
    };
    let histogram = Histogram::new()
        .with_data(values.iter().copied())
        .with_bins(20)
        .with_range(range)
        .with_color("#2b6cb0");
    let plots = vec![Plot::Histogram(histogram)];
    let layout = Layout::auto_from_plots(&plots)
        .with_title("Muon workflow selected lead muon pT")
        .with_x_label("lead muon pT [GeV]")
        .with_y_label("Events")
        .with_width(900.0)
        .with_height(600.0);
    let scene = render_multiple(plots, layout);
    Ok(SvgBackend.render_scene(&scene))
}
