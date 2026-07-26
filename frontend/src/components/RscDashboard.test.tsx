import React from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import RscDashboard from './RscDashboard';

describe('RscDashboard (React Server Components)', () => {
  it('renders loading state initially and then displays server component data', async () => {
    render(<RscDashboard />);

    expect(screen.getByTestId('rsc-loading')).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByTestId('rsc-dashboard')).toBeInTheDocument();
    });

    expect(screen.getByText('Crucible RSC Dashboard (Server Components)')).toBeInTheDocument();
    expect(screen.getByTestId('active-contracts-card')).toHaveTextContent('142');
    expect(screen.getByTestId('health-card')).toHaveTextContent('OPERATIONAL');
  });
});
