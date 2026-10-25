export interface TariffFilterRow {
  providerCode: string;
  provider: string;
  planKey: string;
  planName: string;
  region: string | null;
}

export interface TariffFilters {
  provider: string;
  region: string;
  onlyAeza: boolean;
  query: string;
}

export function filterTariffs<T extends TariffFilterRow>(rows: T[], filters: TariffFilters): T[] {
  const query = filters.query.toLocaleLowerCase();
  return rows.filter(row => (filters.provider === 'all' || row.providerCode === filters.provider)
    && (filters.region === 'all' || (filters.region === '__unknown__' ? row.region === null : row.region === filters.region))
    && (!filters.onlyAeza || row.providerCode === 'aeza')
    && (row.provider + ' ' + row.planName + ' ' + (row.region ?? '') + ' ' + row.planKey)
      .toLocaleLowerCase().includes(query));
}

export function paginateTariffs<T>(rows: T[], page: number, pageSize: number) {
  const pageCount = Math.max(1, Math.ceil(rows.length / pageSize));
  const safePage = Math.min(page, pageCount - 1);
  return {
    pageCount,
    safePage,
    visibleRows: rows.slice(safePage * pageSize, (safePage + 1) * pageSize),
  };
}
