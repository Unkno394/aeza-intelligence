import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { Check, ChevronDown, ChevronUp, Search, UserRound } from 'lucide-react';
import type { Account } from '../../types';
import './account-picker.css';

interface AccountPickerProps {
  accounts: Account[];
  value: string;
  onChange: (value: string) => void;
}

function accountEmailDetail(account: Account) {
  const email = account.email || '';
  const prefix = `${account.name}@`;
  return email.toLocaleLowerCase().startsWith(prefix.toLocaleLowerCase()) ? email.slice(prefix.length) : email;
}

function clientCountLabel(count: number) {
  const lastTwo = count % 100;
  const last = count % 10;
  if (lastTwo >= 11 && lastTwo <= 14) return 'клиентов';
  if (last === 1) return 'клиент';
  if (last >= 2 && last <= 4) return 'клиента';
  return 'клиентов';
}

export function AccountPicker({ accounts, value, onChange }: AccountPickerProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const listId = useId();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [activeId, setActiveId] = useState(value);
  const currentIndex = Math.max(0, accounts.findIndex(account => account.id === value));
  const current = accounts[currentIndex];
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const visibleAccounts = useMemo(() => normalizedQuery
    ? accounts.filter(account => `${account.name} ${account.email} ${account.id}`.toLocaleLowerCase().includes(normalizedQuery))
    : accounts, [accounts, normalizedQuery]);
  const activeIndex = Math.max(0, visibleAccounts.findIndex(account => account.id === activeId));

  const close = () => {
    setOpen(false);
    setQuery('');
  };
  const choose = (account: Account) => {
    onChange(account.id);
    setActiveId(account.id);
    close();
  };
  const selectByOffset = (offset: number) => {
    const next = accounts[currentIndex + offset];
    if (next) choose(next);
  };
  const openPicker = () => {
    setActiveId(value);
    setOpen(true);
    requestAnimationFrame(() => searchRef.current?.focus());
  };
  const moveActive = (offset: number) => {
    if (!visibleAccounts.length) return;
    const nextIndex = (activeIndex + offset + visibleAccounts.length) % visibleAccounts.length;
    setActiveId(visibleAccounts[nextIndex].id);
    requestAnimationFrame(() => document.getElementById(`${listId}-option-${encodeURIComponent(visibleAccounts[nextIndex].id)}`)?.scrollIntoView({ block: 'nearest' }));
  };
  const onSearchKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      moveActive(event.key === 'ArrowDown' ? 1 : -1);
    } else if (event.key === 'Enter' && visibleAccounts[activeIndex]) {
      event.preventDefault();
      choose(visibleAccounts[activeIndex]);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      close();
    }
  };

  useEffect(() => {
    const onPointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) close();
    };
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLocaleLowerCase() === 'k') {
        event.preventDefault();
        setActiveId(value);
        setQuery('');
        setOpen(true);
        requestAnimationFrame(() => searchRef.current?.focus());
      } else if (open && event.key === 'Escape') {
        close();
      }
    };
    if (open) document.addEventListener('pointerdown', onPointerDown);
    window.addEventListener('keydown', onKeyDown);
    return () => {
      if (open) document.removeEventListener('pointerdown', onPointerDown);
      window.removeEventListener('keydown', onKeyDown);
    };
  }, [open, value]);

  if (!current) return null;
  const currentEmail = accountEmailDetail(current);

  return <div className={`client-select account-picker${open ? ' is-open' : ''}`} ref={rootRef} role="group" aria-label="Выбор аккаунта">
    <span className="account-picker-label">Клиент</span>
    <div className="account-picker-main">
      <button type="button" className="account-picker-current" role="combobox" aria-label={`Аккаунт: ${current.name} · ${current.id}`} aria-haspopup="listbox" aria-expanded={open} aria-controls={open ? listId : undefined} onKeyDown={event => { if (event.key === 'ArrowDown' && !open) { event.preventDefault(); openPicker(); } else if (event.key === 'Escape' && open) close(); }} onClick={() => open ? close() : openPicker()}>
        <span className="account-picker-avatar"><UserRound size={18} aria-hidden="true" /></span>
        <span className="account-picker-current-copy"><strong>{current.name}</strong><small>{currentEmail ? `${currentEmail} · ID ${current.id}` : `ID ${current.id}`}</small></span>
        <ChevronDown size={17} className={`account-picker-chevron${open ? ' is-open' : ''}`} aria-hidden="true" />
      </button>
      <div className="account-picker-stepper" role="group" aria-label="Переключить клиента">
        <button type="button" aria-label={`Предыдущий аккаунт · ${currentIndex + 1} из ${accounts.length}`} title="Предыдущий аккаунт" disabled={currentIndex === 0} onClick={() => selectByOffset(-1)}><ChevronUp size={17} /></button>
        <span />
        <button type="button" aria-label={`Следующий аккаунт · ${currentIndex + 1} из ${accounts.length}`} title="Следующий аккаунт" disabled={currentIndex >= accounts.length - 1} onClick={() => selectByOffset(1)}><ChevronDown size={17} /></button>
      </div>
    </div>

    {open && <div className="account-picker-popover">
      <label className="account-picker-search"><Search size={15} aria-hidden="true" /><input ref={searchRef} type="search" value={query} onChange={event => { setQuery(event.currentTarget.value); setActiveId(''); }} onKeyDown={onSearchKeyDown} placeholder="Имя, email или ID клиента…" aria-label="Поиск клиента" aria-controls={listId} aria-activedescendant={visibleAccounts[activeIndex] ? `${listId}-option-${encodeURIComponent(visibleAccounts[activeIndex].id)}` : undefined} /><kbd>⌘K</kbd></label>
      <div className="account-picker-options" id={listId} role="listbox" aria-label="Клиенты">
        {visibleAccounts.length ? visibleAccounts.map(account => {
          const emailDetail = accountEmailDetail(account);
          const selected = account.id === current.id;
          const active = account.id === (activeId || visibleAccounts[0]?.id);
          return <button type="button" role="option" aria-selected={selected} id={`${listId}-option-${encodeURIComponent(account.id)}`} key={account.id} className={`account-picker-option${selected ? ' is-selected' : ''}${active ? ' is-active' : ''}`} onPointerMove={event => { if (event.pointerType === 'mouse') setActiveId(account.id); }} onClick={() => choose(account)}>
            <span className="account-picker-avatar"><UserRound size={16} aria-hidden="true" /></span>
            <span className="account-picker-option-copy"><strong>{account.name}</strong><small>{emailDetail || `ID ${account.id}`}</small></span>
            {selected && <Check size={15} className="account-picker-check" aria-hidden="true" />}
          </button>;
        }) : <p className="account-picker-empty">Клиенты не найдены</p>}
      </div>
      <div className="account-picker-footer"><span>{accounts.length} {clientCountLabel(accounts.length)}</span><span>{currentIndex + 1} из {accounts.length}</span></div>
    </div>}
  </div>;
}
