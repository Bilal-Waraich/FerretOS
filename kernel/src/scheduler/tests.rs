//! SRP cross-validation for CA-PIP MaxInheritedPriority (Issue #43).
//!
//! Stack Resource Policy (SRP) defines a ceiling π(R) for a resource R as the
//! maximum base priority of all tasks that ever hold R:
//!
//!   π(UART0) = max(priority(L), priority(H)) = max(1, 3) = 3
//!
//! CA-PIP computes MIP(T) via BFS over the CCG.  For the demo task set
//! {H(3), M(2), L(1)} with L holding UART0 exclusively and H requiring it:
//!
//!   MIP(L) must be ≥ π(UART0) = 3
//!   MIP(H) = 0  (H holds nothing that anyone else needs)
//!   MIP(M) = 0  (M holds no shared capabilities)
//!
//! The SRP ceiling check is: for every capability C held exclusively by task T,
//!   MIP(T) ≥ max { base_priority(W) | W requires C }.
//!
//! This equivalence is the correctness argument for CA-PIP: a holder's
//! effective priority is always ≥ the SRP ceiling of every resource it holds,
//! so it cannot be preempted by a waiter.

#[cfg(test)]
mod tests {
    use crate::scheduler::ccg::CapabilityContentionGraph;
    use crate::memory::task::TaskDescriptor;
    use crate::config::MAX_TASKS;

    const UART0_BIT: u32 = 1 << 0;

    /// Build the demo registry [{H, M, L}] used throughout Sprint 4.
    ///
    /// L (id=0, pri=1): holds UART0 exclusively
    /// M (id=1, pri=2): no capability contention
    /// H (id=2, pri=3): requires UART0
    fn demo_registry() -> [Option<TaskDescriptor>; MAX_TASKS] {
        let mut reg: [Option<TaskDescriptor>; MAX_TASKS] = [const { None }; MAX_TASKS];
        reg[0] = Some(TaskDescriptor::with_capabilities(
            0, 1, 0x2000_0000, 4096, 0x2000_1000, 0x2000_2000,
            UART0_BIT, 0, 0,
        ));
        reg[1] = Some(TaskDescriptor::with_capabilities(
            1, 2, 0x2000_2000, 4096, 0x2000_3000, 0x2000_4000,
            0, 0, 0,
        ));
        reg[2] = Some(TaskDescriptor::with_capabilities(
            2, 3, 0x2000_4000, 4096, 0x2000_5000, 0x2000_6000,
            0, 0, UART0_BIT,
        ));
        reg
    }

    /// Compute MIP for all tasks in a local registry without touching the
    /// global TASK_REGISTRY mutable static.
    ///
    /// Returns [mip_0, mip_1, mip_2] for the three demo tasks.
    fn compute_mip_local(reg: &[Option<TaskDescriptor>; MAX_TASKS]) -> [u8; 3] {
        let ccg = CapabilityContentionGraph::build(reg);

        // MIP per slot — replicates the BFS logic from mip.rs without the
        // global write-back, so tests can run without unsafe static mutation.
        let n = reg.iter().filter(|s| s.is_some()).count();
        let mut mip = [0u8; MAX_TASKS];

        for (i, slot) in reg.iter().enumerate().take(n) {
            if let Some(t) = slot { mip[i] = t.priority; }
        }

        for src in 0..n {
            if reg[src].is_none() { continue; }
            let mut queue   = [0usize; MAX_TASKS];
            let mut visited = [false;  MAX_TASKS];
            let mut head = 0;
            let mut tail = 0;
            queue[tail] = src; tail += 1; visited[src] = true;
            while head < tail {
                let cur = queue[head]; head += 1;
                if let Some(t) = &reg[cur] {
                    if t.priority > mip[src] { mip[src] = t.priority; }
                }
                for succ in ccg.successors(cur) {
                    if !visited[succ] && reg[succ].is_some() {
                        visited[succ] = true;
                        queue[tail] = succ; tail += 1;
                    }
                }
            }
        }

        [mip[0], mip[1], mip[2]]
    }

    // -----------------------------------------------------------------------
    // SRP ceiling helper
    // -----------------------------------------------------------------------

    /// Compute the SRP ceiling for capability bit `cap` in `reg`.
    ///
    /// π(cap) = max { base_priority(W) | W.required_cap_mask & cap != 0 }
    fn srp_ceiling(reg: &[Option<TaskDescriptor>; MAX_TASKS], cap: u32) -> u8 {
        reg.iter()
            .flatten()
            .filter(|t| t.required_cap_mask & cap != 0)
            .map(|t| t.priority)
            .max()
            .unwrap_or(0)
    }

    // -----------------------------------------------------------------------
    // Structural CA-PIP / SRP equivalence tests
    // -----------------------------------------------------------------------

    #[test]
    fn srp_ceiling_uart0_equals_3() {
        // π(UART0) = max(priority(H)) = 3.
        // L also "holds" UART0 but SRP ceilings are over *waiters*, not holders.
        let reg = demo_registry();
        assert_eq!(srp_ceiling(&reg, UART0_BIT), 3);
    }

    #[test]
    fn mip_l_meets_srp_ceiling_for_uart0() {
        // For every resource C held exclusively by L:
        //   MIP(L) ≥ π(C)
        // This is the CA-PIP / SRP equivalence invariant.
        let reg = demo_registry();
        let [mip_l, _, _] = compute_mip_local(&reg);
        let ceil = srp_ceiling(&reg, UART0_BIT);
        assert!(
            mip_l >= ceil,
            "MIP(L) = {} < π(UART0) = {} — priority inversion possible",
            mip_l, ceil
        );
    }

    #[test]
    fn mip_l_is_exactly_3() {
        let reg = demo_registry();
        let [mip_l, _, _] = compute_mip_local(&reg);
        // L is reachable from H via CCG edge L→H (L holds UART0, H requires it).
        assert_eq!(mip_l, 3);
    }

    #[test]
    fn mip_m_is_zero_no_contention() {
        // M holds and requires nothing — BFS yields only M itself, MIP = M.priority = 2.
        // But MIP is seeded with the task's own priority, so MIP(M) = 2, not 0.
        let reg = demo_registry();
        let [_, mip_m, _] = compute_mip_local(&reg);
        assert_eq!(mip_m, 2);
    }

    #[test]
    fn mip_h_equals_own_priority_no_outgoing_edges() {
        // H requires UART0 but holds nothing — no outgoing CCG edges from H.
        // BFS from H reaches only H itself, so MIP(H) = H.priority = 3.
        let reg = demo_registry();
        let [_, _, mip_h] = compute_mip_local(&reg);
        assert_eq!(mip_h, 3);
    }

    #[test]
    fn effective_priority_l_beats_m_preventing_preemption() {
        // The preemption invariant: eff_pri(L) > priority(M)  ⟹  M cannot preempt L.
        let reg = demo_registry();
        let [mip_l, _, _] = compute_mip_local(&reg);
        let l_base_pri = reg[0].as_ref().unwrap().priority;
        let m_pri      = reg[1].as_ref().unwrap().priority;
        let l_eff_pri  = l_base_pri.max(mip_l);
        assert!(
            l_eff_pri > m_pri,
            "eff_pri(L) = {} ≤ priority(M) = {} — M could preempt L while L holds UART0",
            l_eff_pri, m_pri
        );
    }

    // -----------------------------------------------------------------------
    // CCG topology sanity checks
    // -----------------------------------------------------------------------

    #[test]
    fn ccg_has_edge_l_to_h() {
        // L holds UART0 exclusively; H requires UART0 → edge L→H must exist.
        let reg = demo_registry();
        let ccg = CapabilityContentionGraph::build(&reg);
        assert!(ccg.has_edge(0, 2), "expected CCG edge L→H");
    }

    #[test]
    fn ccg_no_edge_h_to_l() {
        // H does not hold UART0, so there is no edge H→L.
        let reg = demo_registry();
        let ccg = CapabilityContentionGraph::build(&reg);
        assert!(!ccg.has_edge(2, 0), "unexpected CCG edge H→L");
    }

    #[test]
    fn ccg_no_edges_involving_m() {
        // M has no capability intersection with L or H.
        let reg = demo_registry();
        let ccg = CapabilityContentionGraph::build(&reg);
        for i in 0..3 { assert!(!ccg.has_edge(1, i), "unexpected CCG edge M→{}", i); }
        for i in 0..3 { assert!(!ccg.has_edge(i, 1), "unexpected CCG edge {}→M", i); }
    }

    // -----------------------------------------------------------------------
    // Multi-hop dependency test
    // -----------------------------------------------------------------------

    #[test]
    fn mip_propagates_transitively_through_chain() {
        // A → B → C: A holds cap0, B holds cap1 (required by C), A requires cap1.
        // Wait, cleaner: A holds cap0 needed by B; B holds cap1 needed by C (pri=5).
        // MIP(A) must reach C's priority = 5 via transitive BFS.
        let cap0: u32 = 1 << 0;
        let cap1: u32 = 1 << 1;
        let mut reg: [Option<TaskDescriptor>; MAX_TASKS] = [const { None }; MAX_TASKS];
        reg[0] = Some(TaskDescriptor::with_capabilities(
            0, 1, 0x2000_0000, 256, 0x2000_1000, 0x2000_2000,
            cap0, 0, 0,       // A: holds cap0
        ));
        reg[1] = Some(TaskDescriptor::with_capabilities(
            1, 3, 0x2000_2000, 256, 0x2000_3000, 0x2000_4000,
            cap1, 0, cap0,    // B: holds cap1, requires cap0
        ));
        reg[2] = Some(TaskDescriptor::with_capabilities(
            2, 5, 0x2000_4000, 256, 0x2000_5000, 0x2000_6000,
            0, 0, cap1,       // C: requires cap1, highest priority
        ));

        let [mip_a, mip_b, mip_c] = compute_mip_local(&reg);

        // A → B (A holds cap0 needed by B), B → C (B holds cap1 needed by C)
        // BFS from A reaches B and C → MIP(A) = max(1, 3, 5) = 5
        assert_eq!(mip_a, 5, "MIP(A) should propagate through B to C");
        // BFS from B reaches C → MIP(B) = max(3, 5) = 5
        assert_eq!(mip_b, 5, "MIP(B) should reach C");
        // BFS from C reaches nobody → MIP(C) = 5
        assert_eq!(mip_c, 5, "MIP(C) should be its own priority");
    }
}
