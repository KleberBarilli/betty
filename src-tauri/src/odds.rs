//! Faixas de odds, igual a `public/odds-ranges.js`.

use std::sync::LazyLock;

pub struct Faixa {
    pub id: &'static str,
    pub min: f64,
    pub max: Option<f64>,
    pub label: String,
}

const LIMITES: [(f64, &str); 9] =
    [(1.01, "1.01"), (1.10, "1.1"), (1.20, "1.2"), (1.40, "1.4"), (1.60, "1.6"), (1.80, "1.8"), (2.00, "2"), (3.00, "3"), (5.00, "5")];

/// Os IDs persistidos ficam estáveis; o limite superior de cada faixa passa
/// para a faixa anterior (1.11 – 1.20, e não 1.10 – 1.20).
pub static FAIXAS: LazyLock<Vec<Faixa>> = LazyLock::new(|| {
    LIMITES
        .iter()
        .enumerate()
        .map(|(i, &(antigo, id))| {
            let min = if i == 0 { antigo } else { ((antigo * 100.0).round() + 1.0) / 100.0 };
            let max = LIMITES.get(i + 1).map(|l| l.0);
            let label = match max {
                None => format!("{min:.2} ou mais"),
                Some(max) => format!("{min:.2} – {max:.2}"),
            };
            Faixa { id, min, max, label }
        })
        .collect()
});

/// `Math.round(odd * 100 + 1e-9) / 100`, ou 0 quando não há odd válida.
pub fn arredondar(odd: f64) -> f64 {
    if odd.is_finite() && odd > 0.0 { js_round(odd * 100.0 + 1e-9) / 100.0 } else { 0.0 }
}

/// `Math.round` do JavaScript: empate arredonda para +∞. `x - floor(x)` é
/// exato, ao contrário de `floor(x + 0.5)`.
pub fn js_round(x: f64) -> f64 {
    let f = x.floor();
    if x - f >= 0.5 { f + 1.0 } else { f }
}

pub fn faixa(id: &str) -> Option<&'static Faixa> {
    FAIXAS.iter().find(|f| f.id == id)
}

fn dentro(odd: f64, f: &Faixa) -> bool {
    let v = arredondar(odd);
    v >= f.min && f.max.is_none_or(|max| v <= max)
}

pub fn rotulo(odd: f64) -> String {
    FAIXAS.iter().find(|f| dentro(odd, f)).map(|f| f.label.clone()).unwrap_or_else(|| {
        if odd.is_finite() && odd > 0.0 { "Abaixo de 1.01".into() } else { "Odd não disponível".into() }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faixas_iguais_ao_js() {
        let r: Vec<_> = FAIXAS.iter().map(|f| (f.id, f.min, f.max, f.label.as_str())).collect();
        assert_eq!(r[0], ("1.01", 1.01, Some(1.10), "1.01 – 1.10"));
        assert_eq!(r[1], ("1.1", 1.11, Some(1.20), "1.11 – 1.20"));
        assert_eq!(r[8], ("5", 5.01, None, "5.01 ou mais"));
        assert_eq!(rotulo(1.105), "1.11 – 1.20", "1.105 arredonda para 1.11");
        assert_eq!(rotulo(1.0), "Abaixo de 1.01");
        assert_eq!(rotulo(0.0), "Odd não disponível");
        assert_eq!(arredondar(2.345), 2.35);
        assert!(faixa("2").is_some() && faixa("2.0").is_none());
    }
}
