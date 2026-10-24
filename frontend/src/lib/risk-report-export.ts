export type ExportRiskLevel='low'|'medium'|'high'|'limited'|'insufficient';
export interface ExportRiskRow {
 id:string;
 email:string;
 balanceCents:number;
 topupsCents:number;
 monthlyCents:number;
 upcomingCents:number;
 paymentCoverage:number|null;
 score:number;
 level:ExportRiskLevel;
}
export interface ExportRiskReport {rows:ExportRiskRow[];renewalWindowDays:number;createdAt:string}

const headers=['ID','Email','Баланс, EUR','Пополнено всего, EUR','Услуги в месяц, EUR','Платежи в окне, EUR','Покрытие платежей, ×','Risk Score','Уровень риска'];
const riskLabels:Record<ExportRiskLevel,string>={low:'Низкий',medium:'Средний',high:'Высокий',limited:'Ограниченные данные',insufficient:'Недостаточно данных'};
const euro=(cents:number)=>Number((cents/100).toFixed(2));
const fileDate=(report:ExportRiskReport)=>report.createdAt.slice(0,10)||new Date().toISOString().slice(0,10);
const rowsAsValues=(report:ExportRiskReport)=>([[
 'ID','Email','Balance, EUR','Total replenished, EUR','Service cost / month, EUR','Payments in window, EUR','Payment coverage, x','Risk Score','Risk level',
],...report.rows.map(row=>[
 row.id,row.email,euro(row.balanceCents),euro(row.topupsCents),euro(row.monthlyCents),euro(row.upcomingCents),row.paymentCoverage??'',row.score,riskLabels[row.level],
])] as Array<Array<string|number>>);
const xmlEscape=(value:string)=>value.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;').replace(/'/g,'&apos;');
const sqlQuote=(value:string)=>`'${value.replace(/'/g,"''")}'`;
const formatMoney=(cents:number)=>new Intl.NumberFormat('ru-RU',{minimumFractionDigits:2,maximumFractionDigits:2}).format(cents/100);
const formatDate=(value:string)=>new Intl.DateTimeFormat('ru-RU',{dateStyle:'medium',timeStyle:'short'}).format(new Date(value));

function saveBlob(blob:Blob,name:string){
 const url=URL.createObjectURL(blob);
 const link=document.createElement('a');
 link.href=url;link.download=name;link.style.display='none';
 document.body.appendChild(link);link.click();link.remove();
 window.setTimeout(()=>URL.revokeObjectURL(url),1000);
}

export function exportRiskReportCsv(report:ExportRiskReport){
 const values=rowsAsValues(report);
 const csv=values.map(row=>row.map(value=>`"${String(value).replace(/"/g,'""')}"`).join(',')).join('\r\n');
 saveBlob(new Blob(['\uFEFF',csv],{type:'text/csv;charset=utf-8'}),`aeza-risk-report-${fileDate(report)}.csv`);
}

export function exportRiskReportSql(report:ExportRiskReport){
 const statements=[
  `-- Aéza churn risk report; generated ${report.createdAt}; renewal window ${report.renewalWindowDays} days.`,
  '-- Run in a PostgreSQL session. The temporary table remains available until that session closes.',
  'BEGIN;',
  `CREATE TEMP TABLE aeza_churn_risk_report (
  client_id TEXT NOT NULL,
  email TEXT NOT NULL,
  balance_cents BIGINT NOT NULL,
  total_replenished_cents BIGINT NOT NULL,
  monthly_service_cost_cents BIGINT NOT NULL,
  upcoming_payments_cents BIGINT NOT NULL,
  payment_coverage DOUBLE PRECISION,
  risk_score SMALLINT NOT NULL,
  risk_level TEXT NOT NULL
) ON COMMIT PRESERVE ROWS;`,
 ];
 if(report.rows.length){
  const values=report.rows.map(row=>`  (${sqlQuote(row.id)}, ${sqlQuote(row.email)}, ${row.balanceCents}, ${row.topupsCents}, ${row.monthlyCents}, ${row.upcomingCents}, ${row.paymentCoverage===null?'NULL':Number(row.paymentCoverage.toFixed(4))}, ${row.score}, ${sqlQuote(row.level)})`);
  statements.push(`INSERT INTO aeza_churn_risk_report (client_id, email, balance_cents, total_replenished_cents, monthly_service_cost_cents, upcoming_payments_cents, payment_coverage, risk_score, risk_level)\nVALUES\n${values.join(',\n')};`);
 }
 statements.push('COMMIT;','SELECT * FROM aeza_churn_risk_report ORDER BY risk_score DESC, client_id;');
 saveBlob(new Blob([statements.join('\n\n')+'\n'],{type:'application/sql;charset=utf-8'}),`aeza-risk-report-${fileDate(report)}.sql`);
}

function columnName(index:number){
 let value=index+1;let name='';
 while(value){const remainder=(value-1)%26;name=String.fromCharCode(65+remainder)+name;value=Math.floor((value-1)/26);}
 return name;
}
function worksheetCell(reference:string,value:string|number,style?:number){
 const styleAttribute=style===undefined?'':` s="${style}"`;
 if(typeof value==='number'&&Number.isFinite(value))return `<c r="${reference}"${styleAttribute}><v>${value}</v></c>`;
 return `<c r="${reference}" t="inlineStr"${styleAttribute}><is><t xml:space="preserve">${xmlEscape(String(value))}</t></is></c>`;
}
function zipStore(files:Array<{name:string;content:Uint8Array}>){
 const encoder=new TextEncoder();
 const crcTable=new Uint32Array(256);
 for(let n=0;n<256;n++){let c=n;for(let k=0;k<8;k++)c=(c&1)?0xedb88320^(c>>>1):c>>>1;crcTable[n]=c>>>0;}
 const crc32=(bytes:Uint8Array)=>{let crc=0xffffffff;for(const byte of bytes)crc=crcTable[(crc^byte)&0xff]^(crc>>>8);return (crc^0xffffffff)>>>0;};
 const u16=(value:number)=>new Uint8Array([value&0xff,(value>>>8)&0xff]);
 const u32=(value:number)=>new Uint8Array([value&0xff,(value>>>8)&0xff,(value>>>16)&0xff,(value>>>24)&0xff]);
 const concat=(parts:Uint8Array[])=>{const size=parts.reduce((sum,part)=>sum+part.length,0);const result=new Uint8Array(size);let offset=0;for(const part of parts){result.set(part,offset);offset+=part.length;}return result;};
 const localParts:Uint8Array[]=[];const centralParts:Uint8Array[]=[];let offset=0;
 for(const file of files){
  const name=encoder.encode(file.name);const checksum=crc32(file.content);const size=file.content.length;
  const local=concat([u32(0x04034b50),u16(20),u16(0x0800),u16(0),u16(0),u16(0x21),u32(checksum),u32(size),u32(size),u16(name.length),u16(0),name]);
  localParts.push(local,file.content);
  const central=concat([u32(0x02014b50),u16(20),u16(20),u16(0x0800),u16(0),u16(0),u16(0x21),u32(checksum),u32(size),u32(size),u16(name.length),u16(0),u16(0),u16(0),u16(0),u32(0),u32(offset),name]);
  centralParts.push(central);offset+=local.length+size;
 }
 const centralDirectory=concat(centralParts);
 const end=concat([u32(0x06054b50),u16(0),u16(0),u16(files.length),u16(files.length),u32(centralDirectory.length),u32(offset),u16(0)]);
 return concat([...localParts,centralDirectory,end]);
}

export function exportRiskReportXlsx(report:ExportRiskReport){
 const values=rowsAsValues(report);
 const rowsXml=values.map((row,rowIndex)=>`<row r="${rowIndex+1}">${row.map((value,columnIndex)=>{
  const reference=`${columnName(columnIndex)}${rowIndex+1}`;
  if(rowIndex===0)return worksheetCell(reference,value,1);
  const style=columnIndex>=2&&columnIndex<=5?2:columnIndex===6?3:undefined;
  return worksheetCell(reference,value,style);
 }).join('')}</row>`).join('');
 const lastRow=values.length;
 const worksheet=`<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
 <dimension ref="A1:I${lastRow}"/><sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews>
 <sheetFormatPr defaultRowHeight="20"/><cols><col min="1" max="1" width="18" customWidth="1"/><col min="2" max="2" width="34" customWidth="1"/><col min="3" max="6" width="22" customWidth="1"/><col min="7" max="7" width="23" customWidth="1"/><col min="8" max="8" width="14" customWidth="1"/><col min="9" max="9" width="18" customWidth="1"/></cols>
 <sheetData>${rowsXml}</sheetData><autoFilter ref="A1:I${lastRow}"/>
</worksheet>`;
 const styles=`<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts count="1"><numFmt numFmtId="164" formatCode="0.00&quot;×&quot;"/></numFmts><fonts count="2"><font><sz val="11"/><name val="Calibri"/><family val="2"/></font><font><b/><color rgb="FFFFFFFF"/><sz val="11"/><name val="Calibri"/><family val="2"/></font></fonts><fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FF15324B"/><bgColor indexed="64"/></patternFill></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="4"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="2" borderId="0" xfId="0" applyFont="1" applyFill="1"/><xf numFmtId="4" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>`;
 const files=[
  {name:'[Content_Types].xml',content:new TextEncoder().encode(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>`)},
  {name:'_rels/.rels',content:new TextEncoder().encode(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>`)},
  {name:'xl/workbook.xml',content:new TextEncoder().encode(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Клиенты" sheetId="1" r:id="rId1"/></sheets></workbook>`)},
  {name:'xl/_rels/workbook.xml.rels',content:new TextEncoder().encode(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>`)},
  {name:'xl/worksheets/sheet1.xml',content:new TextEncoder().encode(worksheet)},
  {name:'xl/styles.xml',content:new TextEncoder().encode(styles)},
 ];
 saveBlob(new Blob([zipStore(files)],{type:'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'}),`aeza-risk-report-${fileDate(report)}.xlsx`);
}

async function canvasToJpeg(canvas:HTMLCanvasElement){
 const blob=await new Promise<Blob>((resolve,reject)=>canvas.toBlob(value=>value?resolve(value):reject(new Error('Не удалось подготовить страницу PDF.')),'image/jpeg',0.94));
 return new Uint8Array(await blob.arrayBuffer());
}
function drawPdfPage(report:ExportRiskReport,pageRows:ExportRiskRow[],pageNumber:number,pageCount:number){
 const width=842,height=595,scale=2;const canvas=document.createElement('canvas');canvas.width=width*scale;canvas.height=height*scale;
 const context=canvas.getContext('2d');if(!context)throw new Error('Браузер не поддерживает создание PDF.');context.scale(scale,scale);
 context.fillStyle='#f6f9fc';context.fillRect(0,0,width,height);context.fillStyle='#78c9f3';context.fillRect(0,0,width,7);
 context.fillStyle='#10283d';context.font='700 21px Arial, sans-serif';context.fillText('Aéza · Анализ рисков оттока',28,36);
 context.fillStyle='#58748b';context.font='10px Arial, sans-serif';context.fillText(`Сформировано ${formatDate(report.createdAt)} · окно анализа ${report.renewalWindowDays} дней · клиентов в отчёте ${report.rows.length}`,28,56);
 const highRows=report.rows.filter(row=>row.level==='high');const highMonthly=highRows.reduce((sum,row)=>sum+row.monthlyCents,0);
 context.fillStyle='#e5f2fa';context.fillRect(28,69,786,34);context.strokeStyle='#d0e4f0';context.strokeRect(28,69,786,34);
 context.fillStyle='#284b66';context.font='10px Arial, sans-serif';context.fillText(`Высокий риск: ${highRows.length} клиентов    ·    Стоимость их услуг: €${formatMoney(highMonthly)} / мес.`,40,90);
 const columns=[
  {title:'ID',width:60,value:(row:ExportRiskRow)=>row.id},
  {title:'Email',width:198,value:(row:ExportRiskRow)=>row.email},
  {title:'Баланс €',width:72,value:(row:ExportRiskRow)=>formatMoney(row.balanceCents),right:true},
  {title:'Пополнено €',width:82,value:(row:ExportRiskRow)=>formatMoney(row.topupsCents),right:true},
  {title:'Услуги/мес €',width:82,value:(row:ExportRiskRow)=>formatMoney(row.monthlyCents),right:true},
  {title:'Платёж €',width:82,value:(row:ExportRiskRow)=>formatMoney(row.upcomingCents),right:true},
  {title:'Покрытие',width:76,value:(row:ExportRiskRow)=>row.paymentCoverage===null?'—':`${row.paymentCoverage.toFixed(2)}×`,right:true},
  {title:'Score',width:56,value:(row:ExportRiskRow)=>String(row.score),right:true},
  {title:'Уровень',width:78,value:(row:ExportRiskRow)=>riskLabels[row.level]},
 ];
 const left=28,headerY=119,headerHeight=26,rowHeight=18;
 context.fillStyle='#15324b';context.fillRect(left,headerY,786,headerHeight);
 let x=left;
 for(const column of columns){context.save();context.beginPath();context.rect(x+5,headerY,column.width-10,headerHeight);context.clip();context.fillStyle='#f0f8ff';context.font='600 8px Arial, sans-serif';context.textAlign=column.right?'right':'left';context.fillText(column.title,column.right?x+column.width-7:x+7,headerY+17);context.restore();x+=column.width;}
 pageRows.forEach((row,index)=>{
  const y=headerY+headerHeight+index*rowHeight;
  context.fillStyle=index%2?'#ffffff':'#eaf2f8';context.fillRect(left,y,786,rowHeight);
  context.strokeStyle='#d7e4ed';context.beginPath();context.moveTo(left,y+rowHeight);context.lineTo(left+786,y+rowHeight);context.stroke();
  let cellX=left;
  for(const column of columns){
   context.save();context.beginPath();context.rect(cellX+5,y,column.width-10,rowHeight);context.clip();
   context.fillStyle=column.title==='Уровень'?row.level==='high'?'#f47e8e':row.level==='medium'?'#a8d8f1':'#7cceb1':'#27435a';
   context.font='9px Arial, sans-serif';context.textAlign=column.right?'right':'left';context.fillText(column.value(row),column.right?cellX+column.width-7:cellX+7,y+12.5);context.restore();cellX+=column.width;
  }
 });
 context.fillStyle='#6d879b';context.font='9px Arial, sans-serif';context.textAlign='right';context.fillText(`Страница ${pageNumber} из ${pageCount}`,814,580);
 context.textAlign='left';context.fillText('EUR суммы указаны с точностью до цента. Score — индекс риска от 0 до 100.',28,580);
 return canvas;
}
function concatenate(parts:Uint8Array[]){const length=parts.reduce((sum,part)=>sum+part.length,0);const result=new Uint8Array(length);let offset=0;for(const part of parts){result.set(part,offset);offset+=part.length;}return result;}
function buildImagePdf(images:Uint8Array[]){
 const encoder=new TextEncoder();const chunks:Uint8Array[]=[];const offsets=new Map<number,number>();let byteLength=0;
 const write=(part:Uint8Array)=>{chunks.push(part);byteLength+=part.length;};const text=(value:string)=>write(encoder.encode(value));
 const object=(id:number,value:string)=>{offsets.set(id,byteLength);text(`${id} 0 obj\n${value}\nendobj\n`);};
 text('%PDF-1.4\n');
 object(1,'<< /Type /Catalog /Pages 2 0 R >>');
 const pageIds=images.map((_,index)=>5+index*3);
 object(2,`<< /Type /Pages /Count ${images.length} /Kids [${pageIds.map(id=>`${id} 0 R`).join(' ')}] >>`);
 images.forEach((image,index)=>{
  const imageId=3+index*3,contentId=imageId+1,pageId=imageId+2;
  offsets.set(imageId,byteLength);text(`${imageId} 0 obj\n<< /Type /XObject /Subtype /Image /Width 1684 /Height 1190 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length ${image.length} >>\nstream\n`);write(image);text('\nendstream\nendobj\n');
  const stream=encoder.encode('q 842 0 0 595 0 0 cm /Im0 Do Q');
  offsets.set(contentId,byteLength);text(`${contentId} 0 obj\n<< /Length ${stream.length} >>\nstream\n`);write(stream);text('\nendstream\nendobj\n');
  object(pageId,`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 842 595] /Resources << /XObject << /Im0 ${imageId} 0 R >> >> /Contents ${contentId} 0 R >>`);
 });
 const maxId=2+images.length*3;const xrefOffset=byteLength;text(`xref\n0 ${maxId+1}\n0000000000 65535 f \n`);
 for(let id=1;id<=maxId;id++)text(`${String(offsets.get(id)??0).padStart(10,'0')} 00000 n \n`);
 text(`trailer\n<< /Size ${maxId+1} /Root 1 0 R >>\nstartxref\n${xrefOffset}\n%%EOF`);
 return new Blob([concatenate(chunks)],{type:'application/pdf'});
}

export async function exportRiskReportPdf(report:ExportRiskReport){
 const pageSize=23;const pageCount=Math.max(1,Math.ceil(report.rows.length/pageSize));const images:Uint8Array[]=[];
 for(let page=0;page<pageCount;page++){
  const pageRows=report.rows.slice(page*pageSize,(page+1)*pageSize);
  images.push(await canvasToJpeg(drawPdfPage(report,pageRows,page+1,pageCount)));
 }
 saveBlob(buildImagePdf(images),`aeza-risk-report-${fileDate(report)}.pdf`);
}
