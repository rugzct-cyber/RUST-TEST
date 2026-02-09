//! Quadratic Scaling-In Layer Calculator
//!
//! Computes entry layers with:
//! - Spread triggers linearly spaced between `spread_min` and `spread_max`
//! - Quantities following a quadratic progression (i+1)² so larger sizes
//!   are allocated to wider spreads (more attractive opportunities)

/// A single entry layer with its spread trigger and quantity
#[derive(Debug, Clone)]
pub struct EntryLayer {
    /// Layer index (0-based)
    pub index: usize,
    /// Spread threshold (%) that activates this layer
    pub spread_trigger: f64,
    /// Quantity to trade for this layer
    pub quantity: f64,
}

/// Calculate entry layers with quadratic size scaling
///
/// # Arguments
/// - `spread_min`: Lower spread bound (e.g. 0.30%)
/// - `spread_max`: Upper spread bound (e.g. 0.70%)
/// - `total_size`: Total position size to distribute across all layers
/// - `num_layers`: Number of layers (typically 5)
///
/// # Panics
/// Panics if `num_layers == 0`
///
/// # Returns
/// Vec of `EntryLayer` sorted by ascending spread trigger.
/// Layer 0 has the smallest quantity, layer N-1 the largest.
pub fn calculate_entry_layers(
    spread_min: f64,
    spread_max: f64,
    total_size: f64,
    num_layers: usize,
) -> Vec<EntryLayer> {
    assert!(num_layers > 0, "num_layers must be > 0");

    // Edge case: single layer gets everything at spread_min
    if num_layers == 1 {
        return vec![EntryLayer {
            index: 0,
            spread_trigger: spread_min,
            quantity: total_size,
        }];
    }

    // Quadratic weights: (i+1)^2 → [1, 4, 9, 16, 25] for 5 layers
    let weights: Vec<f64> = (0..num_layers).map(|i| ((i + 1) as f64).powi(2)).collect();
    let weight_sum: f64 = weights.iter().sum();

    // Spread step between layers
    let spread_step = (spread_max - spread_min) / (num_layers - 1) as f64;

    (0..num_layers)
        .map(|i| EntryLayer {
            index: i,
            spread_trigger: spread_min + i as f64 * spread_step,
            quantity: (weights[i] / weight_sum) * total_size,
        })
        .collect()
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_5_layers_correct_spread_triggers() {
        let layers = calculate_entry_layers(0.30, 0.70, 1.0, 5);
        assert_eq!(layers.len(), 5);

        let expected_triggers = [0.30, 0.40, 0.50, 0.60, 0.70];
        for (layer, expected) in layers.iter().zip(expected_triggers.iter()) {
            assert!(
                (layer.spread_trigger - expected).abs() < 1e-10,
                "Layer {} trigger {:.4} != expected {:.4}",
                layer.index,
                layer.spread_trigger,
                expected
            );
        }
    }

    #[test]
    fn test_5_layers_correct_quantities() {
        let total = 1.0;
        let layers = calculate_entry_layers(0.30, 0.70, total, 5);

        // Weights: [1, 4, 9, 16, 25], sum = 55
        let expected_qtys = [1.0 / 55.0, 4.0 / 55.0, 9.0 / 55.0, 16.0 / 55.0, 25.0 / 55.0];
        for (layer, expected) in layers.iter().zip(expected_qtys.iter()) {
            assert!(
                (layer.quantity - expected).abs() < 1e-10,
                "Layer {} qty {:.6} != expected {:.6}",
                layer.index,
                layer.quantity,
                expected
            );
        }

        // Sum of quantities must equal total
        let qty_sum: f64 = layers.iter().map(|l| l.quantity).sum();
        assert!(
            (qty_sum - total).abs() < 1e-10,
            "Total quantity {:.10} != {:.10}",
            qty_sum,
            total
        );
    }

    #[test]
    fn test_quantities_are_monotonically_increasing() {
        let layers = calculate_entry_layers(0.30, 0.70, 10.0, 5);
        for i in 1..layers.len() {
            assert!(
                layers[i].quantity > layers[i - 1].quantity,
                "Layer {} qty {:.6} should be > layer {} qty {:.6}",
                i,
                layers[i].quantity,
                i - 1,
                layers[i - 1].quantity
            );
        }
    }

    #[test]
    fn test_single_layer() {
        let layers = calculate_entry_layers(0.50, 0.50, 2.5, 1);
        assert_eq!(layers.len(), 1);
        assert!((layers[0].spread_trigger - 0.50).abs() < 1e-10);
        assert!((layers[0].quantity - 2.5).abs() < 1e-10);
    }

    #[test]
    fn test_custom_total_size() {
        let total = 5.0;
        let layers = calculate_entry_layers(0.20, 0.80, total, 5);
        let qty_sum: f64 = layers.iter().map(|l| l.quantity).sum();
        assert!(
            (qty_sum - total).abs() < 1e-10,
            "Total quantity {:.10} != {:.10}",
            qty_sum,
            total
        );
    }

    #[test]
    fn test_two_layers() {
        let layers = calculate_entry_layers(0.30, 0.70, 1.0, 2);
        assert_eq!(layers.len(), 2);
        // Triggers: [0.30, 0.70]
        assert!((layers[0].spread_trigger - 0.30).abs() < 1e-10);
        assert!((layers[1].spread_trigger - 0.70).abs() < 1e-10);
        // Weights: [1, 4], sum=5 → [0.2, 0.8]
        assert!((layers[0].quantity - 0.2).abs() < 1e-10);
        assert!((layers[1].quantity - 0.8).abs() < 1e-10);
    }

    #[test]
    #[should_panic(expected = "num_layers must be > 0")]
    fn test_zero_layers_panics() {
        calculate_entry_layers(0.30, 0.70, 1.0, 0);
    }
}
