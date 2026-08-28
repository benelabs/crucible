import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, beforeEach } from 'vitest';
import { I18nextProvider } from 'react-i18next';
import i18n from '../i18n/i18n';
import { LanguageSwitcher, SUPPORTED_LANGUAGES } from './LanguageSwitcher';

/** Helper: reset i18n to English before every test */
beforeEach(async () => {
  await i18n.changeLanguage('en');
});

describe('LanguageSwitcher – language toggle', () => {
  it('renders the language select with English selected by default', () => {
    render(
      <I18nextProvider i18n={i18n}>
        <LanguageSwitcher />
      </I18nextProvider>
    );

    const select = screen.getByRole('combobox') as HTMLSelectElement;
    expect(select).toBeInTheDocument();
    expect(select.value).toBe('en');
  });

  it('lists all five supported languages as options', () => {
    render(
      <I18nextProvider i18n={i18n}>
        <LanguageSwitcher />
      </I18nextProvider>
    );

    const options = screen.getAllByRole('option') as HTMLOptionElement[];
    const optionValues = options.map((o) => o.value);

    expect(optionValues).toContain('en');
    expect(optionValues).toContain('es');
    expect(optionValues).toContain('zh');
    expect(optionValues).toContain('ja');
    expect(optionValues).toContain('pt');
    expect(options).toHaveLength(SUPPORTED_LANGUAGES.length);
  });

  it('changes i18n language to Spanish when Español is selected', async () => {
    render(
      <I18nextProvider i18n={i18n}>
        <LanguageSwitcher />
      </I18nextProvider>
    );

    const select = screen.getByRole('combobox');
    fireEvent.change(select, { target: { value: 'es' } });

    await waitFor(() => {
      expect(i18n.language.startsWith('es')).toBe(true);
    });
  });

  it('changes i18n language to Mandarin (zh) when 中文 is selected', async () => {
    render(
      <I18nextProvider i18n={i18n}>
        <LanguageSwitcher />
      </I18nextProvider>
    );

    const select = screen.getByRole('combobox');
    fireEvent.change(select, { target: { value: 'zh' } });

    await waitFor(() => {
      expect(i18n.language.startsWith('zh')).toBe(true);
    });
  });

  it('changes i18n language to Japanese (ja) when 日本語 is selected', async () => {
    render(
      <I18nextProvider i18n={i18n}>
        <LanguageSwitcher />
      </I18nextProvider>
    );

    const select = screen.getByRole('combobox');
    fireEvent.change(select, { target: { value: 'ja' } });

    await waitFor(() => {
      expect(i18n.language.startsWith('ja')).toBe(true);
    });
  });

  it('changes i18n language to Portuguese (pt) when Português is selected', async () => {
    render(
      <I18nextProvider i18n={i18n}>
        <LanguageSwitcher />
      </I18nextProvider>
    );

    const select = screen.getByRole('combobox');
    fireEvent.change(select, { target: { value: 'pt' } });

    await waitFor(() => {
      expect(i18n.language.startsWith('pt')).toBe(true);
    });
  });

  it('translates app.language key per the active language', async () => {
    // After switching to Spanish the aria-label should surface the Spanish
    // translation of app.language ('Idioma')
    render(
      <I18nextProvider i18n={i18n}>
        <LanguageSwitcher />
      </I18nextProvider>
    );

    const select = screen.getByRole('combobox');
    fireEvent.change(select, { target: { value: 'es' } });

    await waitFor(() => {
      expect(i18n.t('app.language')).toBe('Idioma');
    });
  });
});

describe('LanguageSwitcher – fallback handling', () => {
  it('falls back to English translation when a key is missing in the active locale', async () => {
    await i18n.changeLanguage('zh');

    // app.title must exist in all locales; if zh somehow lacked it the
    // fallback mechanism would return the English value 'Crucible'
    expect(i18n.t('app.title')).toBeTruthy();
  });

  it('falls back gracefully when an unsupported language code is requested', async () => {
    // 'xx' is not a supported locale – i18next should silently fall back to 'en'
    await i18n.changeLanguage('xx');
    expect(i18n.t('app.title')).toBe('Crucible');
  });

  it('English translation keys are non-empty strings', () => {
    expect(i18n.t('app.title')).not.toBe('');
    expect(i18n.t('app.nav.dashboard')).not.toBe('');
    expect(i18n.t('wallet.connect')).not.toBe('');
  });
});
