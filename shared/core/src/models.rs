//! Whisper (ggml) model catalog and device-based recommendation.
//!
//! Sizes and RAM figures are approximate and shown to the user as such.
//! No checksums are hard-coded: integrity is checked against the hash that the
//! hosting server reports at download time and a local sidecar hash afterwards.

#[derive(Debug, Clone, Copy)]
pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub file: &'static str,
    /// Approximate download / disk size in MB.
    pub size_mb: u32,
    /// Approximate RAM while transcribing, in MB.
    pub ram_mb: u32,
    pub quantized: bool,
    pub note: &'static str,
}

pub const CATALOG: &[ModelSpec] = &[
    ModelSpec { id: "tiny", name: "Tiny", file: "ggml-tiny.bin", size_mb: 75, ram_mb: 400, quantized: false,
        note: "Fastest, lowest accuracy. English is usable; Tamil is very poor. Not recommended." },
    ModelSpec { id: "base", name: "Base", file: "ggml-base.bin", size_mb: 142, ram_mb: 500, quantized: false,
        note: "Fast. English is decent; Tamil is poor. Not recommended for Tamil." },
    ModelSpec { id: "small-q5_1", name: "Small (quantized)", file: "ggml-small-q5_1.bin", size_mb: 181, ram_mb: 700, quantized: true,
        note: "Smallest size with usable English; Tamil is fair at best. For weak PCs only." },
    ModelSpec { id: "small", name: "Small", file: "ggml-small.bin", size_mb: 466, ram_mb: 1000, quantized: false,
        note: "Good English, fair Tamil." },
    ModelSpec { id: "medium-q5_0", name: "Medium (quantized)", file: "ggml-medium-q5_0.bin", size_mb: 539, ram_mb: 1700, quantized: true,
        note: "Good Tamil and English. Best choice for mid-range PCs." },
    ModelSpec { id: "large-v3-turbo-q5_0", name: "Large v3 Turbo (quantized)", file: "ggml-large-v3-turbo-q5_0.bin", size_mb: 574, ram_mb: 1900, quantized: true,
        note: "Fast and very good English. Its trimmed decoder may be weaker on Tamil than full Large v3." },
    ModelSpec { id: "large-v3-turbo", name: "Large v3 Turbo", file: "ggml-large-v3-turbo.bin", size_mb: 1620, ram_mb: 3000, quantized: false,
        note: "Fast, high English accuracy; Tamil may trail full Large v3." },
    ModelSpec { id: "large-v3-q5_0", name: "Large v3 (quantized)", file: "ggml-large-v3-q5_0.bin", size_mb: 1080, ram_mb: 3500, quantized: true,
        note: "Very high accuracy for Tamil and English with little loss from quantization. Slow without a GPU." },
    ModelSpec { id: "large-v3", name: "Large v3", file: "ggml-large-v3.bin", size_mb: 3100, ram_mb: 5000, quantized: false,
        note: "Highest accuracy available here for Tamil and English. Needs about 5 GB free RAM and is slow on CPU." },
];

const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/";

pub fn find(id: &str) -> Option<&'static ModelSpec> {
    CATALOG.iter().find(|m| m.id == id)
}

pub fn download_url(spec: &ModelSpec) -> String {
    format!("{}{}", BASE_URL, spec.file)
}

/// Recommend a model for a machine. Accuracy first: Tamil needs a Medium-class model or larger, so the
/// recommendation only drops below that when the PC genuinely cannot run it.
pub fn recommend(total_ram_mb: u64, cpu_cores: usize) -> &'static ModelSpec {
    let id = if total_ram_mb < 4096 {
        "small-q5_1"
    } else if total_ram_mb < 8192 || cpu_cores < 4 {
        "medium-q5_0"
    } else if total_ram_mb < 16384 || cpu_cores < 8 {
        "large-v3-q5_0"
    } else {
        "large-v3"
    };
    find(id).expect("recommended id exists in catalog")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_unique_and_files_end_with_bin() {
        for (i, a) in CATALOG.iter().enumerate() {
            assert!(a.file.ends_with(".bin"));
            for b in &CATALOG[i + 1..] {
                assert_ne!(a.id, b.id);
            }
        }
    }

    #[test]
    fn recommendation_scales_with_hardware() {
        assert_eq!(recommend(3000, 2).id, "small-q5_1");
        assert_eq!(recommend(6000, 8).id, "medium-q5_0");
        assert_eq!(recommend(32000, 2).id, "medium-q5_0");
        assert_eq!(recommend(12000, 8).id, "large-v3-q5_0");
        assert_eq!(recommend(32000, 16).id, "large-v3");
    }

    #[test]
    fn url_shape() {
        let u = download_url(find("small").unwrap());
        assert!(u.starts_with("https://") && u.ends_with("ggml-small.bin"));
    }
}
