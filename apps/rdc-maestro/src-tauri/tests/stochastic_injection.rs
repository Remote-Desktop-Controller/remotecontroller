//! Testes para o escalonamento estocástico de injeção de prompts no debate multi-agente

use rdc_maestro_lib::orchestrator::compute_stochastic_jitter_schedule;
use std::time::Duration;

#[test]
fn generates_increasing_jitter_delays_for_providers() {
    let count = 5;
    let min_delay = 300;
    let max_delay = 1200;

    let schedule = compute_stochastic_jitter_schedule(count, min_delay, max_delay);

    assert_eq!(schedule.len(), count);
    // O primeiro provedor deve disparar imediatamente (0ms)
    assert_eq!(schedule[0], Duration::from_millis(0));

    // Os provedores subsequentes devem ter atrasos crescentes e dentro da janela estocástica
    let mut prev = Duration::from_millis(0);
    for delay in &schedule[1..] {
        assert!(
            *delay > prev,
            "Atraso deve ser estritamente crescente para escalonamento temporal"
        );
        let delta = (*delay - prev).as_millis() as u64;
        assert!(
            delta >= min_delay && delta <= max_delay,
            "Delta {}ms fora dos limites [{}ms, {}ms]",
            delta,
            min_delay,
            max_delay
        );
        prev = *delay;
    }
}

#[test]
fn handles_zero_and_single_provider_gracefully() {
    let zero = compute_stochastic_jitter_schedule(0, 300, 1200);
    assert!(zero.is_empty());

    let single = compute_stochastic_jitter_schedule(1, 300, 1200);
    assert_eq!(single.len(), 1);
    assert_eq!(single[0], Duration::from_millis(0));
}
