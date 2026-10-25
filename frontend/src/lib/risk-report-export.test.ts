import test from 'node:test';
import assert from 'node:assert/strict';
import {
  exportRiskReportCsv, exportRiskReportPdf, exportRiskReportSql, exportRiskReportXlsx,
  type ExportRiskReport,
} from './risk-report-export';
import {selectCustomerRows, type CustomerTableRow} from '../features/risk/customerTable';

const report: ExportRiskReport = {
  createdAt: '2026-10-25T07:00:00.000Z', renewalWindowDays: 14,
  rows: [{
    id: "O'Neil", email: 'user,"quoted"@example.test', balanceCents: 12345,
    topupsCents: 23456, monthlyCents: 34567, upcomingCents: 45678,
    paymentCoverage: 1.25, score: 81, level: 'high',
  }],
};

function withDownloadMocks<T>(body: (readDownloads: () => Promise<{name: string; blob: Blob}[]>) => Promise<T>): Promise<T> {
  const downloads: {name: string; blob: Blob}[] = [];
  const urls = new Map<string, Blob>();
  let nextUrl = 0;
  const anchor = {
    href: '', download: '', style: {display: ''},
    click() { const blob = urls.get(this.href); assert.ok(blob); downloads.push({name: this.download, blob}); },
    remove() {},
  };
  const context = Object.fromEntries([
    'scale','fillRect','strokeRect','fillText','save','restore','beginPath','rect','clip','moveTo','lineTo','stroke',
  ].map(method => [method, () => {}]));
  const canvas = {
    width: 0, height: 0,
    getContext: () => context,
    toBlob: (callback: BlobCallback) => callback(new Blob(['jpeg-bytes'], {type:'image/jpeg'})),
  };
  const fakeDocument = {
    createElement: (tag: string) => tag === 'canvas' ? canvas : anchor,
    body: {appendChild: () => {}},
  } as unknown as Document;
  const oldDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
  const oldWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const oldCreate = Object.getOwnPropertyDescriptor(URL, 'createObjectURL');
  const oldRevoke = Object.getOwnPropertyDescriptor(URL, 'revokeObjectURL');
  Object.defineProperty(globalThis, 'document', {configurable:true, value:fakeDocument});
  Object.defineProperty(globalThis, 'window', {configurable:true, value:{setTimeout:() => 0}});
  Object.defineProperty(URL, 'createObjectURL', {configurable:true, value:(blob: Blob) => {const url=`blob:test/${++nextUrl}`;urls.set(url,blob);return url;}});
  Object.defineProperty(URL, 'revokeObjectURL', {configurable:true, value:() => {}});
  const read = async () => downloads;
  return body(read).finally(() => {
    for (const [object, key, descriptor] of [
      [globalThis,'document',oldDocument], [globalThis,'window',oldWindow],
      [URL,'createObjectURL',oldCreate], [URL,'revokeObjectURL',oldRevoke],
    ] as const) {
      if (descriptor) Object.defineProperty(object, key, descriptor);
      else Reflect.deleteProperty(object, key);
    }
  });
}

test('CSV and SQL exports preserve supplied rows, cents, and quoted customer data', async () => {
  await withDownloadMocks(async read => {
    exportRiskReportCsv(report);
    exportRiskReportSql(report);
    const downloads = await read();
    assert.equal(downloads.length, 2);
    assert.match(downloads[0].name, /2026-10-25\.csv$/);
    const csvBytes = new Uint8Array(await downloads[0].blob.arrayBuffer());
    assert.deepEqual([...csvBytes.slice(0,3)], [0xef,0xbb,0xbf]);
    const csv = new TextDecoder().decode(csvBytes.slice(3));
    assert.ok(csv.includes('"O\'Neil","user,""quoted""@example.test","123.45","234.56","345.67","456.78","1.25","81","Высокий"'));
    assert.ok(csv.includes('"123.45","234.56","345.67","456.78"'));
    assert.match(downloads[1].name, /2026-10-25\.sql$/);
    const sql = await downloads[1].blob.text();
    assert.ok(sql.includes("('O''Neil', 'user,\"quoted\"@example.test', 12345, 23456, 34567, 45678, 1.25, 81, 'high')"));
    assert.ok(sql.includes('ON COMMIT PRESERVE ROWS'));
  });
});

test('Excel export emits a valid ZIP-based workbook with numeric cells and a filter range', async () => {
  await withDownloadMocks(async read => {
    exportRiskReportXlsx(report);
    const [download] = await read();
    assert.match(download.name, /2026-10-25\.xlsx$/);
    const bytes = new Uint8Array(await download.blob.arrayBuffer());
    assert.deepEqual([...bytes.slice(0,4)], [0x50,0x4b,0x03,0x04]);
    const archive = new TextDecoder().decode(bytes);
    assert.ok(archive.includes('[Content_Types].xml'));
    assert.ok(archive.includes('user,&quot;quoted&quot;@example.test'));
    assert.ok(archive.includes('<v>123.45</v>'));
    assert.ok(archive.includes('<autoFilter ref="A1:I2"/>'));
  });
});

test('PDF export writes a PDF with the supplied report and page metadata', async () => {
  await withDownloadMocks(async read => {
    await exportRiskReportPdf(report);
    const [download] = await read();
    assert.match(download.name, /2026-10-25\.pdf$/);
    const pdf = new TextDecoder().decode(await download.blob.arrayBuffer());
    assert.ok(pdf.startsWith('%PDF-1.4'));
    assert.ok(pdf.includes('/Type /Pages /Count 1'));
    assert.ok(pdf.includes('startxref'));
    assert.ok(pdf.endsWith('%%EOF'));
  });
});

test('export rows follow the active filters and include every match across all pages', async () => {
  const customers: CustomerTableRow[] = [
    {id:'2',name:'Aéza East',email:'east@aeza.test',balanceCents:10,topupsCents:20,score:80,level:'high'},
    {id:'10',name:'Aéza West',email:'west@aeza.test',balanceCents:30,topupsCents:40,score:90,level:'high'},
    {id:'11',name:'Aéza Low',email:'low@aeza.test',balanceCents:50,topupsCents:60,score:10,level:'low'},
    {id:'100',name:'Other high',email:'other@example.test',balanceCents:70,topupsCents:80,score:75,level:'high'},
  ];
  const selected = selectCustomerRows(customers, {
    search:'aeza', risk:'high', sort:'id', direction:1, page:1, limit:1,
  });
  assert.deepEqual(selected.ordered.map(row => row.id), ['2','10']);
  assert.deepEqual(selected.pageRows.map(row => row.id), ['10']);
  const filteredReport: ExportRiskReport = {
    ...report,
    rows:selected.ordered.map(row => ({
      ...report.rows[0], id:row.id, email:row.email, score:row.score, level:row.level,
    })),
  };
  await withDownloadMocks(async read => {
    exportRiskReportCsv(filteredReport);
    const csv = await (await read())[0].blob.text();
    assert.ok(csv.includes('east@aeza.test'));
    assert.ok(csv.includes('west@aeza.test'));
    assert.ok(!csv.includes('low@aeza.test'));
    assert.ok(!csv.includes('other@example.test'));
  });
});
