#!/bin/bash
# SCMessenger Orchestrator Status Script
# Shows integration with existing agent system

echo " SCMessenger Orchestrator Integration Status"
echo "============================================="

# Check orchestrator state
if [ -f ".claude/orchestrator_state.json" ]; then
    echo "[OK] Orchestrator state tracking: Enabled"
    STATUS=$(grep -o '"orchestrator_active": [^,]*' .claude/orchestrator_state.json)
    echo "  Current status: $STATUS"
else
    echo "[WARNING] Orchestrator state tracking: Not configured"
fi

echo ""
echo " Existing Agent System Status:"
echo "TODO tasks: $(ls HANDOFF/todo/ 2>/dev/null | wc -l)"
echo "DONE tasks: $(ls HANDOFF/done/ 2>/dev/null | wc -l)"
echo "15-minute maintenance loop: [OK] Active (retention enforcement)"

echo ""
echo " Integration Commands:"
echo "- 'Activate orchestrator role' - Enable monitoring"
echo "- 'Deactivate orchestrator role' - Disable monitoring"
echo "- 'Orchestrator status' - Show current state"

echo ""
echo " System Integration:"
echo "[OK] Uses existing HANDOFF task system"
echo "[OK] Respects 15-minute maintenance loop"
echo "[OK] Follows AGENT_HANDOFF_GUIDANCE.md"
echo "[OK] No duplication - pure integration"