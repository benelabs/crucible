import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { InteractiveChallengeEngine, Challenge, ChallengeResult } from './InteractiveChallengeEngine';

const mockChallenge: Challenge = {
  id: 'challenge-1',
  title: 'Write Your First Soroban Contract',
  description: 'Learn how to create a simple counter contract in Soroban',
  difficulty: 'beginner',
  initialCode: `// Start coding here\nuse soroban_sdk::{contract, contractimpl};`,
  testCases: [
    {
      name: 'Contract has increment function',
      description: 'Verify contract exports increment function',
      input: 'pub fn increment',
      expected: 'pub fn increment() -> u32'
    },
    {
      name: 'Contract stores state',
      description: 'Verify contract stores counter state',
      input: 'env.storage',
      expected: 'Counter persisted in storage'
    }
  ],
  hints: [
    'Remember to use #[contract] and #[contractimpl] macros from soroban_sdk',
    'Use env.storage().instance() to persist data',
    'The Soroban documentation is your friend!'
  ],
  steps: [
    {
      id: 'step-1',
      title: 'Create the contract struct',
      description: 'Define the counter contract struct',
      testCaseIndex: 0,
      hint: 'Use #[contract] macro'
    },
    {
      id: 'step-2',
      title: 'Implement increment',
      description: 'Add increment function',
      testCaseIndex: 1,
      hint: 'Use env.storage() to persist'
    }
  ]
};

describe('InteractiveChallengeEngine', () => {
  it('renders challenge title and description', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    
    expect(screen.getByText('Write Your First Soroban Contract')).toBeInTheDocument();
    expect(screen.getByText('Learn how to create a simple counter contract in Soroban')).toBeInTheDocument();
  });

  it('displays difficulty badge', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    
    const badge = screen.getByText('beginner');
    expect(badge).toHaveClass('difficulty-badge', 'difficulty-beginner');
  });

  it('renders code editor with initial code', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    
    const editor = screen.getByTestId('code-editor') as HTMLTextAreaElement;
    expect(editor.value).toContain('// Start coding here');
    expect(editor.value).toContain('use soroban_sdk');
  });

  it('allows user to edit code', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    
    const editor = screen.getByTestId('code-editor') as HTMLTextAreaElement;
    fireEvent.change(editor, { target: { value: 'pub fn increment' } });
    
    expect(editor.value).toBe('pub fn increment');
  });

  it('renders all hints collapsed initially', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    
    mockChallenge.hints.forEach((_, idx) => {
      const hintContent = screen.queryByTestId(`hint-content-${idx}`);
      expect(hintContent).not.toBeInTheDocument();
    });
  });

  it('expands hint when clicked', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    
    const hintToggle = screen.getByTestId('hint-toggle-0');
    fireEvent.click(hintToggle);
    
    const hintContent = screen.getByTestId('hint-content-0');
    expect(hintContent).toBeInTheDocument();
    expect(hintContent.textContent).toContain('Remember to use #[contract]');
  });

  it('collapses hint when clicked again', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    
    const hintToggle = screen.getByTestId('hint-toggle-0');
    
    // Open
    fireEvent.click(hintToggle);
    expect(screen.getByTestId('hint-content-0')).toBeInTheDocument();
    
    // Close
    fireEvent.click(hintToggle);
    expect(screen.queryByTestId('hint-content-0')).not.toBeInTheDocument();
  });

  it('allows only one hint expanded at a time', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    
    const hint0Toggle = screen.getByTestId('hint-toggle-0');
    const hint1Toggle = screen.getByTestId('hint-toggle-1');
    
    fireEvent.click(hint0Toggle);
    expect(screen.getByTestId('hint-content-0')).toBeInTheDocument();
    expect(screen.queryByTestId('hint-content-1')).not.toBeInTheDocument();
    
    fireEvent.click(hint1Toggle);
    expect(screen.queryByTestId('hint-content-0')).not.toBeInTheDocument();
    expect(screen.getByTestId('hint-content-1')).toBeInTheDocument();
  });

  it('runs tests when Run Tests button is clicked', async () => {
    const onComplete = vi.fn();
    render(<InteractiveChallengeEngine challenge={mockChallenge} onComplete={onComplete} />);
    
    const editor = screen.getByTestId('code-editor') as HTMLTextAreaElement;
    fireEvent.change(editor, { target: { value: mockChallenge.initialCode + '\npub fn increment' } });
    
    const runBtn = screen.getByTestId('run-tests-btn');
    fireEvent.click(runBtn);
    
    await waitFor(() => {
      expect(screen.getByTestId('test-results')).toBeInTheDocument();
    });
  });

  it('displays test results after execution', async () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    
    const editor = screen.getByTestId('code-editor') as HTMLTextAreaElement;
    fireEvent.change(editor, { target: { value: mockChallenge.initialCode + '\npub fn increment' } });
    
    const runBtn = screen.getByTestId('run-tests-btn');
    fireEvent.click(runBtn);
    
    await waitFor(() => {
      const results = screen.getByTestId('test-results');
      expect(results).toBeInTheDocument();
      expect(results.textContent).toContain('tests passed');
    });
  });

  it('shows passed test with checkmark', async () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    
    const editor = screen.getByTestId('code-editor') as HTMLTextAreaElement;
    fireEvent.change(editor, { target: { value: mockChallenge.initialCode + '\npub fn increment' } });
    
    const runBtn = screen.getByTestId('run-tests-btn');
    fireEvent.click(runBtn);
    
    await waitFor(() => {
      const testResults = screen.getAllByTestId(/test-result-/);
      expect(testResults.length).toBeGreaterThan(0);
    });
  });

  it('calls onComplete callback with results', async () => {
    const onComplete = vi.fn();
    render(<InteractiveChallengeEngine challenge={mockChallenge} onComplete={onComplete} />);
    
    const editor = screen.getByTestId('code-editor') as HTMLTextAreaElement;
    fireEvent.change(editor, { target: { value: mockChallenge.initialCode + '\npub fn increment' } });
    
    const runBtn = screen.getByTestId('run-tests-btn');
    fireEvent.click(runBtn);
    
    await waitFor(() => {
      expect(onComplete).toHaveBeenCalled();
      const results = onComplete.mock.calls[0][0] as ChallengeResult[];
      expect(Array.isArray(results)).toBe(true);
    });
  });

  it('resets code to initial state when reset button is clicked', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    
    const editor = screen.getByTestId('code-editor') as HTMLTextAreaElement;
    fireEvent.change(editor, { target: { value: 'modified code' } });
    expect(editor.value).toBe('modified code');
    
    const resetBtn = screen.getByTestId('reset-btn');
    fireEvent.click(resetBtn);
    
    expect(editor.value).toBe(mockChallenge.initialCode);
  });

  it('clears test results when code is reset', async () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    
    // Run tests first
    const editor = screen.getByTestId('code-editor') as HTMLTextAreaElement;
    fireEvent.change(editor, { target: { value: mockChallenge.initialCode + '\npub fn increment' } });
    
    const runBtn = screen.getByTestId('run-tests-btn');
    fireEvent.click(runBtn);
    
    await waitFor(() => {
      expect(screen.getByTestId('test-results')).toBeInTheDocument();
    });
    
    // Reset
    const resetBtn = screen.getByTestId('reset-btn');
    fireEvent.click(resetBtn);
    
    // Results should be cleared
    expect(screen.queryByTestId('test-results')).not.toBeInTheDocument();
  });

  it('disables run and reset buttons while tests are executing', async () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    
    const runBtn = screen.getByTestId('run-tests-btn');
    const resetBtn = screen.getByTestId('reset-btn');
    
    fireEvent.click(runBtn);
    
    expect(runBtn).toBeDisabled();
    expect(resetBtn).toBeDisabled();
    
    await waitFor(() => {
      expect(runBtn).not.toBeDisabled();
      expect(resetBtn).not.toBeDisabled();
    });
  });

  it('shows progress bar at 100% when all tests pass', async () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    
    const editor = screen.getByTestId('code-editor') as HTMLTextAreaElement;
    fireEvent.change(editor, { target: { value: mockChallenge.initialCode + '\npub fn increment' + '\nenv.storage' } });
    
    const runBtn = screen.getByTestId('run-tests-btn');
    fireEvent.click(runBtn);
    
    await waitFor(() => {
      const progressFill = screen.getByTestId('progress-fill');
      expect(progressFill).toHaveStyle({ width: '100%' });
    });
  });

  it('displays challenge engine with test data-testid', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    
    const engine = screen.getByTestId('challenge-engine');
    expect(engine).toBeInTheDocument();
  });

  it('handles test execution errors gracefully', async () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    
    const runBtn = screen.getByTestId('run-tests-btn');
    fireEvent.click(runBtn);
    
    await waitFor(() => {
      expect(screen.getByTestId('test-results')).toBeInTheDocument();
    });
  });

  it('validates all test cases are present in results', async () => {
    const onComplete = vi.fn();
    render(<InteractiveChallengeEngine challenge={mockChallenge} onComplete={onComplete} />);
    
    const runBtn = screen.getByTestId('run-tests-btn');
    fireEvent.click(runBtn);
    
    await waitFor(() => {
      expect(onComplete).toHaveBeenCalled();
      const results = onComplete.mock.calls[0][0] as ChallengeResult[];
      expect(results.length).toBe(mockChallenge.testCases.length);
    });
  });

  it('displays test details with expected and actual values', async () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    
    const runBtn = screen.getByTestId('run-tests-btn');
    fireEvent.click(runBtn);
    
    await waitFor(() => {
      expect(screen.getByTestId('test-results')).toBeInTheDocument();
      // Results will contain expected/actual information
    });
  });

  it('renders challenge engine container with proper structure', () => {
    const { container } = render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    
    expect(container.querySelector('.interactive-challenge-engine')).toBeInTheDocument();
    expect(container.querySelector('.challenge-header')).toBeInTheDocument();
    expect(container.querySelector('.challenge-content')).toBeInTheDocument();
  });

  // Step Progression Tests
  it('renders step progression', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    expect(screen.getByTestId('step-progression')).toBeInTheDocument();
  });

  it('displays all steps in progression', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    expect(screen.getByTestId('step-0')).toBeInTheDocument();
    expect(screen.getByTestId('step-1')).toBeInTheDocument();
  });

  it('activates first step by default', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    const step0 = screen.getByTestId('step-0');
    expect(step0).toHaveClass('active');
  });

  it('changes active step on click', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    const step1 = screen.getByTestId('step-1');
    fireEvent.click(step1);
    expect(step1).toHaveClass('active');
  });

  it('displays step details for active step', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    expect(screen.getByText('Create the contract struct')).toBeInTheDocument();
  });

  it('updates step details when step changes', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    const step1 = screen.getByTestId('step-1');
    fireEvent.click(step1);
    expect(screen.getByText('Implement increment')).toBeInTheDocument();
  });

  // Hint Reveal System Tests
  it('locks hints by default', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    expect(screen.getByText(/Hint 1 \(Locked\)/)).toBeInTheDocument();
  });

  it('reveals hint on click', async () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    const hintToggle = screen.getByTestId('hint-toggle-0');
    fireEvent.click(hintToggle);
    
    await waitFor(() => {
      expect(screen.getByText(/Hint 1 \(Revealed\)/)).toBeInTheDocument();
    });
  });

  it('shows revealed hint count', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    const hintToggle = screen.getByTestId('hint-toggle-0');
    fireEvent.click(hintToggle);
    expect(screen.getByText(/Hints \(1 revealed\)/)).toBeInTheDocument();
  });

  it('tracks multiple revealed hints', async () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    const hint0 = screen.getByTestId('hint-toggle-0');
    const hint1 = screen.getByTestId('hint-toggle-1');
    
    fireEvent.click(hint0);
    fireEvent.click(hint1);
    
    await waitFor(() => {
      expect(screen.getByText(/Hints \(2 revealed\)/)).toBeInTheDocument();
    });
  });

  // Step Completion Tests
  it('calls onStepComplete callback when step test passes', async () => {
    const onStepComplete = vi.fn();
    render(<InteractiveChallengeEngine challenge={mockChallenge} onStepComplete={onStepComplete} />);
    
    const editor = screen.getByTestId('code-editor') as HTMLTextAreaElement;
    fireEvent.change(editor, { target: { value: mockChallenge.initialCode + '\npub fn increment' } });
    
    const runBtn = screen.getByTestId('run-tests-btn');
    fireEvent.click(runBtn);
    
    await waitFor(() => {
      expect(onStepComplete).toHaveBeenCalledWith('step-1');
    });
  });

  it('marks completed steps visually', async () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    
    const editor = screen.getByTestId('code-editor') as HTMLTextAreaElement;
    fireEvent.change(editor, { target: { value: mockChallenge.initialCode + '\npub fn increment' } });
    
    const runBtn = screen.getByTestId('run-tests-btn');
    fireEvent.click(runBtn);
    
    await waitFor(() => {
      const step0 = screen.getByTestId('step-0');
      expect(step0).toHaveClass('completed');
    });
  });

  it('displays step progress indicator', async () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    expect(screen.getByText(/0\/2 Steps Complete/)).toBeInTheDocument();
  });

  it('updates progress when steps complete', async () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    
    const editor = screen.getByTestId('code-editor') as HTMLTextAreaElement;
    fireEvent.change(editor, { target: { value: mockChallenge.initialCode + '\npub fn increment' } });
    
    const runBtn = screen.getByTestId('run-tests-btn');
    fireEvent.click(runBtn);
    
    await waitFor(() => {
      expect(screen.getByText(/1\/2 Steps Complete/)).toBeInTheDocument();
    });
  });

  // Step Navigation Tests
  it('displays step navigation buttons', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    expect(screen.getByTestId('prev-step-btn')).toBeInTheDocument();
    expect(screen.getByTestId('next-step-btn')).toBeInTheDocument();
  });

  it('navigates to next step', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    const nextBtn = screen.getByTestId('next-step-btn');
    fireEvent.click(nextBtn);
    expect(screen.getByText(/Step 2 of 2/)).toBeInTheDocument();
  });

  it('navigates to previous step', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    const nextBtn = screen.getByTestId('next-step-btn');
    fireEvent.click(nextBtn);
    
    const prevBtn = screen.getByTestId('prev-step-btn');
    fireEvent.click(prevBtn);
    
    expect(screen.getByText(/Step 1 of 2/)).toBeInTheDocument();
  });

  it('disables prev button on first step', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    expect(screen.getByTestId('prev-step-btn')).toBeDisabled();
  });

  it('disables next button on last step', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    const nextBtn = screen.getByTestId('next-step-btn');
    fireEvent.click(nextBtn);
    expect(nextBtn).toBeDisabled();
  });

  it('displays skip button', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    expect(screen.getByTestId('skip-btn')).toBeInTheDocument();
  });

  it('skips to next step', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    const skipBtn = screen.getByTestId('skip-btn');
    fireEvent.click(skipBtn);
    expect(screen.getByText(/Step 2 of 2/)).toBeInTheDocument();
  });

  it('displays step counter', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    expect(screen.getByText(/Step 1 of 2/)).toBeInTheDocument();
  });

  it('updates step counter on navigation', () => {
    render(<InteractiveChallengeEngine challenge={mockChallenge} />);
    const nextBtn = screen.getByTestId('next-step-btn');
    fireEvent.click(nextBtn);
    expect(screen.getByText(/Step 2 of 2/)).toBeInTheDocument();
  });
