import test from 'node:test';
import assert from 'node:assert/strict';
import type {CustomerTableOptions,CustomerTableRow} from './customerTable';
import {selectCustomerRows} from './customerTable';

const rows:CustomerTableRow[]=[
  {id:'10',name:'Zoe Ten',email:'zoe@example.test',balanceCents:500,topupsCents:2000,score:50,level:'medium'},
  {id:'2',name:'Alice Two',email:'alice@example.test',balanceCents:1000,topupsCents:1000,score:90,level:'high'},
  {id:'100',name:'Bob Hundred',email:'bob@example.test',balanceCents:500,topupsCents:2000,score:50,level:'low'},
  {id:'11',name:'Carol Eleven',email:'carol@example.test',balanceCents:250,topupsCents:3000,score:20,level:'limited'},
];
const options=(overrides:Partial<CustomerTableOptions>={}):CustomerTableOptions=>({
  search:'',risk:'all',sort:'id',direction:1,page:0,limit:20,...overrides,
});
const ids=(values:CustomerTableRow[])=>values.map(row=>row.id);

test('ID sort is numeric in both directions, including values wider than machine integers',()=>{
  assert.deepEqual(ids(selectCustomerRows(rows,options()).ordered),['2','10','11','100']);
  assert.deepEqual(ids(selectCustomerRows(rows,options({direction:-1})).ordered),['100','11','10','2']);
  const wide=[{...rows[0],id:'9007199254740993'},{...rows[1],id:'9007199254740992'}];
  assert.deepEqual(ids(selectCustomerRows(wide,options()).ordered),['9007199254740992','9007199254740993']);
});

test('email, balance, replenishments, and score support ascending and descending order',()=>{
  assert.deepEqual(ids(selectCustomerRows(rows,options({sort:'email'})).ordered),['2','100','11','10']);
  assert.deepEqual(ids(selectCustomerRows(rows,options({sort:'email',direction:-1})).ordered),['10','11','100','2']);
  assert.deepEqual(ids(selectCustomerRows(rows,options({sort:'balanceCents'})).ordered),['11','10','100','2']);
  assert.deepEqual(ids(selectCustomerRows(rows,options({sort:'balanceCents',direction:-1})).ordered),['2','10','100','11']);
  assert.deepEqual(ids(selectCustomerRows(rows,options({sort:'topupsCents'})).ordered),['2','10','100','11']);
  assert.deepEqual(ids(selectCustomerRows(rows,options({sort:'topupsCents',direction:-1})).ordered),['11','10','100','2']);
  assert.deepEqual(ids(selectCustomerRows(rows,options({sort:'score'})).ordered),['11','10','100','2']);
  assert.deepEqual(ids(selectCustomerRows(rows,options({sort:'score',direction:-1})).ordered),['2','10','100','11']);
});

test('search and risk filters combine; empty results and optional-looking row fields are safe',()=>{
  assert.deepEqual(ids(selectCustomerRows(rows,options({search:'ALICE',risk:'high'})).ordered),['2']);
  assert.deepEqual(ids(selectCustomerRows(rows,options({search:'example.test',risk:'limited'})).ordered),['11']);
  assert.deepEqual(selectCustomerRows(rows,options({search:'nobody'})).ordered,[]);
  assert.deepEqual(selectCustomerRows(rows,options({search:'  alice  '})).ordered,[rows[1]]);
});

test('equal values preserve original order and page selection is clamped and stable',()=>{
  assert.deepEqual(ids(selectCustomerRows(rows,options({sort:'score',direction:1})).ordered),['11','10','100','2']);
  const result=selectCustomerRows(rows,options({sort:'id',page:1,limit:2}));
  assert.deepEqual(ids(result.pageRows),['11','100']);
  assert.equal(result.totalPages,2);
  assert.equal(selectCustomerRows(rows,options({page:99,limit:2})).currentPage,1);
  assert.equal(selectCustomerRows([],options()).totalPages,1);
});

test('export input uses all filtered and sorted rows, not only the visible page',()=>{
  const result=selectCustomerRows(rows,options({risk:'all',page:3,limit:1}));
  assert.deepEqual(ids(result.ordered),['2','10','11','100']);
  assert.deepEqual(ids(result.pageRows),['100']);
});
