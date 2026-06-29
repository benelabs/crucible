import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { EventListenerDashboard } from './EventListenerDashboard';

describe('EventListenerDashboard', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-05-31T14:00:00.000Z'));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('renders listener status, metrics, feed, and details', () => {
    render(<EventListenerDashboard />);

    expect(screen.getByText('Event Listener')).toBeInTheDocument();
    expect(screen.getByTestId('listener-status')).toHaveTextContent('connected');
    expect(screen.getByText('Events')).toBeInTheDocument();
    expect(screen.getByTestId('event-feed')).toBeInTheDocument();
    expect(screen.getByTestId('event-details')).toHaveTextContent('transfer');
  });

  it('filters events by severity', () => {
    render(<EventListenerDashboard />);

    fireEvent.click(screen.getByTestId('severity-critical'));

    expect(screen.getByTestId('severity-critical')).toHaveClass('active');
    expect(screen.getByTestId('event-feed')).toHaveTextContent('admin_call');
    expect(screen.getByTestId('event-feed')).not.toHaveTextContent('transfer');
  });

  it('filters events by search query', () => {
    render(<EventListenerDashboard />);

    fireEvent.change(screen.getByTestId('event-search'), {
      target: { value: 'escrow' },
    });

    expect(screen.getByTestId('event-feed')).toHaveTextContent('escrow_release');
    expect(screen.getByTestId('event-feed')).not.toHaveTextContent('admin_call');
  });

  it('shows an empty state when filters match no events', () => {
    render(<EventListenerDashboard />);

    fireEvent.change(screen.getByTestId('event-search'), {
      target: { value: 'does-not-exist' },
    });

    expect(screen.getByTestId('empty-feed')).toBeInTheDocument();
    expect(screen.getByTestId('event-details')).toHaveTextContent('No event selected.');
  });

  it('selects an event and updates the detail panel', () => {
    render(<EventListenerDashboard />);

    fireEvent.click(screen.getByTestId('event-row-0000859036408881149-0000000004'));

    expect(screen.getByTestId('event-details')).toHaveTextContent('admin_call');
    expect(screen.getByTestId('event-details')).toHaveTextContent('governance.admin');
  });

  it('appends live events while connected', () => {
    render(<EventListenerDashboard />);

    act(() => {
      vi.advanceTimersByTime(1800);
    });

    expect(screen.getByTestId('event-row-0000859036408882001-0000000001')).toBeInTheDocument();
    expect(screen.getByTestId('event-feed')).toHaveTextContent('approval');
  });

  it('pauses and resumes the live feed', () => {
    render(<EventListenerDashboard />);

    fireEvent.click(screen.getByTestId('pause-feed'));
    expect(screen.getByTestId('listener-status')).toHaveTextContent('paused');

    act(() => {
      vi.advanceTimersByTime(1800);
    });
    expect(screen.queryByTestId('event-row-0000859036408882001-0000000001')).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId('resume-feed'));
    expect(screen.getByTestId('listener-status')).toHaveTextContent('connected');

    act(() => {
      vi.advanceTimersByTime(1800);
    });
    expect(screen.getByTestId('event-row-0000859036408882001-0000000001')).toBeInTheDocument();
  });

  it('disconnects the listener', () => {
    render(<EventListenerDashboard />);

    fireEvent.click(screen.getByTestId('disconnect-feed'));

    expect(screen.getByTestId('listener-status')).toHaveTextContent('disconnected');
    expect(screen.getByTestId('resume-feed')).toBeInTheDocument();
  });
});
