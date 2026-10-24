export interface Account { id: string; name: string; email: string; balanceCents: number; joinedAt?: string; totalReplenishedCents?: number }
export interface Service { id: string; accountId: string; name: string; region: string|null; priceCents: number; renewalPriceCents?: number; paymentTermSeconds?: number; renewsAt: string; autoRenew: boolean; status: 'active' | 'suspended' | 'cancelled' }
export interface Transaction { id: string; accountId: string; serviceId?: string; amountCents: number; type: 'topup' | 'payment'; status: 'completed' | 'failed'; createdAt: string }
export interface Dataset { accounts: Account[]; services: Service[]; transactions: Transaction[]; asOf: string; snapshots: {date: string; accountId: string; score: number}[] }
export interface Settings { weights: { balance: number; renewal: number; autoRenew: number; status: number; transactions: number }; mediumThreshold: number; highThreshold: number; renewalWindowDays: number; historicalMinDays: number }
export type RiskLevel = 'low' | 'medium' | 'high';
export interface RiskResult { score: number; level: RiskLevel; factors: {key: keyof Settings['weights']; label: string; severity: number; points: number; explanation: string}[]; upcomingCents: number; monthlyCents: number; topupsCents: number }
