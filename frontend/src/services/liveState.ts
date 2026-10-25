import type { Dataset, RiskResult, Settings, SettingsMetadata } from '../types';
import type { AccountRisk, ServerMessage } from './live';

export interface AnalyzerLiveState {
  data: Dataset | null;
  settings: Settings | null;
  settingsMetadata: SettingsMetadata | null;
  risks: Record<string, RiskResult> | null;
}

export const emptyAnalyzerLiveState: AnalyzerLiveState = {
  data: null,
  settings: null,
  settingsMetadata: null,
  risks: null,
};

const byAccount = (rows: AccountRisk[]) =>
  Object.fromEntries(rows.map(({ accountId, ...risk }) => [accountId, risk])) as Record<string, RiskResult>;

export function applyLiveMessage(state: AnalyzerLiveState, message: ServerMessage): AnalyzerLiveState {
  switch (message.type) {
    case 'snapshot':
      return {
        data: message.dataset,
        settings: message.settings,
        settingsMetadata: message.settingsMetadata,
        risks: byAccount(message.risks),
      };
    case 'dataset':
      return { ...state, data: message.dataset };
    case 'settings':
      return { ...state, settings: message.settings, settingsMetadata: message.settingsMetadata };
    case 'scores': {
      const risks = { ...(state.risks ?? {}) };
      for (const id of message.removed) delete risks[id];
      return { ...state, risks: { ...risks, ...byAccount(message.changed) } };
    }
    case 'ack':
    case 'error':
      return state;
  }
}
