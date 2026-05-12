use crate::chess::{piece_value, Board, Move, MoveList, PieceKind};
use crate::nnue_runtime::NnueRuntime;
use crate::tablebase;
use crate::time_manager::TimeBudget;
use crate::tt::{Bound, TranspositionTable};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

const MATE_VALUE: i32 = 30_000;
const ENDGAME_MATERIAL_THRESHOLD: i32 = 2_000;
const MAX_PLY: usize = 64;
const SEE_RANGE: i32 = 100;
const MATE_SCORE_BUFFER: i32 = MAX_PLY as i32;

fn mated_score_from_ply(ply: usize) -> i32 {
    -MATE_VALUE + ply as i32
}

fn to_tt_score(score: i32, ply: usize) -> i32 {
    if score >= MATE_VALUE - MATE_SCORE_BUFFER {
        score + ply as i32
    } else if score <= -MATE_VALUE + MATE_SCORE_BUFFER {
        score - ply as i32
    } else {
        score
    }
}

fn from_tt_score(score: i32, ply: usize) -> i32 {
    if score >= MATE_VALUE - MATE_SCORE_BUFFER {
        score - ply as i32
    } else if score <= -MATE_VALUE + MATE_SCORE_BUFFER {
        score + ply as i32
    } else {
        score
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SearchStatistics {
    pub nodes: u64,
}

#[derive(Clone)]
struct SearchState {
    killer_moves: [[Option<Move>; 2]; MAX_PLY],
    history: [[u32; 64]; 64],
    optimal_deadline: Option<Instant>,
    hard_deadline: Option<Instant>,
    abort_flag: Option<Arc<AtomicBool>>,
    stopped: bool,
    repetition: RepetitionTracker,
    endgame: bool,
}

fn move_indices(mv: Move) -> (usize, usize) {
    let from = mv.from.rank() as usize * 8 + mv.from.file() as usize;
    let to = mv.to.rank() as usize * 8 + mv.to.file() as usize;
    (from, to)
}

impl SearchState {
    fn new(
        deadlines: Option<(Instant, Instant)>,
        abort_flag: Option<Arc<AtomicBool>>,
        repetition_history: &[u64],
        current_hash: u64,
        material: i32,
    ) -> Self {
        let mut repetition = RepetitionTracker::new(repetition_history);
        if repetition.history.last().copied() != Some(current_hash) {
            repetition.history.push(current_hash);
        }
        let endgame = material <= ENDGAME_MATERIAL_THRESHOLD;
        Self {
            killer_moves: [[None; 2]; MAX_PLY],
            history: [[0; 64]; 64],
            optimal_deadline: deadlines.map(|d| d.0),
            hard_deadline: deadlines.map(|d| d.1),
            abort_flag,
            stopped: false,
            repetition,
            endgame,
        }
    }

    fn killer_moves(&self, ply: usize) -> [Option<Move>; 2] {
        self.killer_moves[ply.min(MAX_PLY - 1)]
    }

    fn record_killer(&mut self, ply: usize, mv: Move) {
        let idx = ply.min(MAX_PLY - 1);
        if self.killer_moves[idx].contains(&Some(mv)) {
            return;
        }
        self.killer_moves[idx][1] = self.killer_moves[idx][0];
        self.killer_moves[idx][0] = Some(mv);
    }

    fn record_history(&mut self, mv: Move, depth: u8) {
        let (from, to) = move_indices(mv);
        const HISTORY_LIMIT: u32 = 1_000_000;
        let bonus = (depth as u32) * (depth as u32);
        let entry = &mut self.history[from][to];
        *entry = entry.saturating_add(bonus).min(HISTORY_LIMIT);
    }

    fn history_score(&self, mv: Move) -> i32 {
        let (from, to) = move_indices(mv);
        self.history[from][to] as i32
    }

    fn should_stop(&mut self) -> bool {
        if self.stopped {
            return true;
        }
        if let Some(flag) = &self.abort_flag {
            if flag.load(Ordering::Relaxed) {
                self.stopped = true;
                return true;
            }
        }
        let now = Instant::now();
        if let Some(opt_deadline) = self.optimal_deadline {
            if now >= opt_deadline {
                self.stopped = true;
                return true;
            }
        }
        if let Some(hard_deadline) = self.hard_deadline {
            if now >= hard_deadline {
                self.stopped = true;
                return true;
            }
        }
        false
    }

    fn push_repetition(&mut self, hash: u64, irreversible: bool) {
        self.repetition.push(hash, irreversible);
    }

    fn pop_repetition(&mut self) {
        self.repetition.pop();
    }

    fn is_threefold(&self, hash: u64) -> bool {
        self.repetition.is_threefold(hash)
    }

    fn in_endgame(&self) -> bool {
        self.endgame
    }
}

#[derive(Clone)]
struct RepetitionTracker {
    history: Vec<u64>,
    reset_stack: Vec<usize>,
    current_reset: usize,
}

impl RepetitionTracker {
    fn new(base: &[u64]) -> Self {
        Self {
            history: base.to_vec(),
            reset_stack: Vec::new(),
            current_reset: 0,
        }
    }

    fn push(&mut self, hash: u64, irreversible: bool) {
        self.reset_stack.push(self.current_reset);
        self.history.push(hash);
        if irreversible {
            self.current_reset = self.history.len().saturating_sub(1);
        }
    }

    fn pop(&mut self) {
        if !self.history.is_empty() {
            self.history.pop();
        }
        if let Some(prev) = self.reset_stack.pop() {
            self.current_reset = prev;
        }
    }

    fn is_threefold(&self, hash: u64) -> bool {
        self.history[self.current_reset..]
            .iter()
            .filter(|&&h| h == hash)
            .count()
            >= 3
    }
}

#[derive(Clone, Copy)]
struct SearchWindow {
    alpha: i32,
    beta: i32,
}

impl SearchWindow {
    fn new(alpha: i32, beta: i32) -> Self {
        Self { alpha, beta }
    }

    fn flipped(self) -> Self {
        SearchWindow::new(-self.beta, -self.alpha)
    }
}

struct SearchContext<'a> {
    board: &'a mut Board,
    tt: &'a mut TranspositionTable,
    stats: &'a mut SearchStatistics,
    state: &'a mut SearchState,
    nnue_runner: &'a NnueRuntime,
}

impl SearchStatistics {
    fn record_node(&mut self) {
        self.nodes += 1;
    }
}

#[derive(Debug, Clone)]
pub struct SearchReport {
    pub best_move: Option<Move>,
    pub depth: u8,
    pub stats: SearchStatistics,
    pub elapsed: Duration,
}

impl SearchReport {
    pub fn nps(&self) -> u64 {
        let elapsed_ms = self.elapsed.as_millis() as u64;
        if elapsed_ms == 0 {
            self.stats.nodes
        } else {
            (self.stats.nodes * 1000) / elapsed_ms.max(1)
        }
    }
}

pub fn search_best_move(
    board: &mut Board,
    tt: &mut TranspositionTable,
    max_depth: u8,
    time_budget: Option<TimeBudget>,
    nnue_runner: &NnueRuntime,
    abort_flag: Option<Arc<AtomicBool>>,
    repetition_history: &[u64],
    root_hint: Option<Move>,
) -> SearchReport {
    let start = Instant::now();
    let mut stats = SearchStatistics::default();
    let mut root_moves = MoveList::new();
    board.legal_moves_into(&mut root_moves);
    if root_moves.is_empty() {
        return SearchReport {
            best_move: None,
            depth: 0,
            stats: SearchStatistics::default(),
            elapsed: Duration::from_millis(0),
        };
    }

    if let Some(mate_move) = immediate_mate_move(board, &root_moves) {
        return SearchReport {
            best_move: Some(mate_move),
            depth: 0,
            stats,
            elapsed: start.elapsed(),
        };
    }

    if let Some(best_move) = tablebase::best_root_move(board) {
        return SearchReport {
            best_move: Some(best_move),
            depth: 0,
            stats,
            elapsed: start.elapsed(),
        };
    }

    let deadlines = time_budget.map(|b| (start + b.optimal, start + b.maximum));
    let mut state = SearchState::new(
        deadlines,
        abort_flag.clone(),
        repetition_history,
        board.hash(),
        board.material_count(),
    );

    let mut best_move = root_hint;
    let mut completed_depth = 0;
    let mut last_score = 0;
    const ASP_WINDOW: i32 = 100;

    {
        let mut ctx = SearchContext {
            board,
            tt,
            stats: &mut stats,
            state: &mut state,
            nnue_runner,
        };

        'search: for current_depth in 1..=max_depth {
            if ctx.state.should_stop() {
                break;
            }
            let mut window = if current_depth > 1 {
                SearchWindow::new(
                    (last_score - ASP_WINDOW).max(-MATE_VALUE),
                    (last_score + ASP_WINDOW).min(MATE_VALUE),
                )
            } else {
                SearchWindow::new(-MATE_VALUE, MATE_VALUE)
            };

            loop {
                let (candidate_move, score) = search_single_depth(
                    &mut ctx,
                    current_depth,
                    &mut root_moves,
                    best_move,
                    window,
                );
                if ctx.state.should_stop() {
                    break 'search;
                }

                if score <= window.alpha && window.alpha > -MATE_VALUE {
                    window.alpha = window.alpha.saturating_sub(ASP_WINDOW * 2);
                    continue;
                } else if score >= window.beta && window.beta < MATE_VALUE {
                    window.beta = window.beta.saturating_add(ASP_WINDOW * 2);
                    continue;
                } else {
                    best_move = candidate_move;
                    last_score = score;
                    completed_depth = current_depth;
                    break;
                }
            }
        }
    }

    SearchReport {
        best_move,
        depth: completed_depth,
        stats,
        elapsed: start.elapsed(),
    }
}

fn search_single_depth(
    ctx: &mut SearchContext,
    depth: u8,
    root_moves: &mut MoveList,
    preferred: Option<Move>,
    window: SearchWindow,
) -> (Option<Move>, i32) {
    let tt_move = ctx
        .tt
        .probe(ctx.board.hash())
        .and_then(|entry| entry.best_move);
    let killers = ctx.state.killer_moves(0);
    order_moves(
        ctx.board, root_moves, preferred, tt_move, killers, ctx.state,
    );

    let mut best_move = None;
    let mut best_score = -MATE_VALUE;
    let mut bounds = window;
    let mut move_count = 0;

    for &mv in root_moves.iter() {
        move_count += 1;
        let undo = ctx.board.make_move(mv);
        let irreversible = undo.moving_piece.kind == PieceKind::Pawn
            || undo.captured_piece.is_some();
        ctx.state.push_repetition(ctx.board.hash(), irreversible);
        let mut score;
        if move_count == 1 {
            score = -negamax(
                ctx,
                depth.saturating_sub(1),
                SearchWindow::new(-bounds.beta, -bounds.alpha),
                1,
                true,
            );
        } else {
            let narrow =
                SearchWindow::new(-bounds.alpha.saturating_add(1), -bounds.alpha);
            score = -negamax(ctx, depth.saturating_sub(1), narrow, 1, false);
            if score > bounds.alpha && score < bounds.beta {
                score = -negamax(
                    ctx,
                    depth.saturating_sub(1),
                    SearchWindow::new(-bounds.beta, -bounds.alpha),
                    1,
                    true,
                );
            }
        }
        ctx.state.pop_repetition();
        ctx.board.unmake_move(undo);
        if score > best_score {
            best_score = score;
            best_move = Some(mv);
        }
        bounds.alpha = bounds.alpha.max(best_score);
        if bounds.alpha >= bounds.beta {
            break;
        }
    }

    (best_move, best_score)
}

fn negamax(
    ctx: &mut SearchContext,
    depth: u8,
    mut window: SearchWindow,
    ply: usize,
    is_pv: bool,
) -> i32 {
    if ctx.state.should_stop() {
        return 0;
    }
    ctx.stats.record_node();
    if ctx.state.is_threefold(ctx.board.hash()) {
        return 0;
    }
    if ctx.board.halfmove_clock >= 100 {
        return 0;
    }
    if let Some(score) = tablebase::probe_score(ctx.board, ply) {
        return score;
    }

    let alpha_orig = window.alpha;
    let beta_orig = window.beta;

    let key = ctx.board.hash();
    let mut tt_move = None;
    if let Some(entry) = ctx.tt.probe(key) {
        tt_move = entry.best_move;
        if entry.depth >= depth {
            let value = from_tt_score(entry.value, ply);
            match entry.bound {
                Bound::Exact => return value,
                Bound::Lower => window.alpha = window.alpha.max(value),
                Bound::Upper => window.beta = window.beta.min(value),
            }
            if window.alpha >= window.beta {
                return value;
            }
        }
    }

    if depth == 0 {
        return quiescence(ctx, window, ply);
    }

    const NULL_MOVE_REDUCTION: u8 = 2;
    if depth > NULL_MOVE_REDUCTION + 1
        && !ctx.board.is_in_check(ctx.board.active_color)
        && !ctx.state.in_endgame()
    {
        let undo = ctx.board.make_null_move();
        let score = -negamax(
            ctx,
            depth - 1 - NULL_MOVE_REDUCTION,
            SearchWindow::new(-window.beta, -window.beta + 1),
            ply + 1,
            false,
        );
        ctx.board.unmake_null_move(undo);
        if score >= window.beta {
            return score;
        }
    }

    let mut moves = MoveList::new();
    ctx.board.legal_moves_into(&mut moves);
    let killers = ctx.state.killer_moves(ply);
    order_moves(ctx.board, &mut moves, None, tt_move, killers, ctx.state);
    if moves.is_empty() {
        if ctx.board.is_in_check(ctx.board.active_color) {
            return mated_score_from_ply(ply);
        } else {
            return 0;
        }
    }

    let mut best_value = -MATE_VALUE;
    let mut best_move = None;
    let mut move_count = 0;
    for &mv in moves.as_slice() {
        move_count += 1;
        let undo = ctx.board.make_move(mv);
        let is_capture = undo.captured_piece.is_some();
        if is_capture && depth > 1 && ctx.board.static_exchange_eval(mv) < -SEE_RANGE
        {
            ctx.board.unmake_move(undo);
            continue;
        }
        let irreversible = is_capture || undo.moving_piece.kind == PieceKind::Pawn;
        ctx.state.push_repetition(ctx.board.hash(), irreversible);
        let gives_check = ctx.board.is_in_check(ctx.board.active_color);
        let mut child_depth = depth.saturating_sub(1);
        let mut reduced = false;
        let child_is_pv = is_pv && move_count == 1;
        if depth >= 3
            && move_count > 1
            && !is_capture
            && !gives_check
            && !child_is_pv
            && !ctx.state.in_endgame()
        {
            reduced = true;
            let reduction = 1 + (move_count > 6) as u8 + (depth > 5) as u8;
            child_depth = child_depth.saturating_sub(reduction);
        }
        let mut score;
        if child_is_pv {
            score = -negamax(ctx, child_depth, window.flipped(), ply + 1, true);
        } else {
            let narrow =
                SearchWindow::new(-window.alpha.saturating_add(1), -window.alpha);
            score = -negamax(ctx, child_depth, narrow, ply + 1, false);
            if reduced && score > window.alpha {
                child_depth = depth.saturating_sub(1);
                let retry =
                    SearchWindow::new(-window.alpha.saturating_add(1), -window.alpha);
                score = -negamax(ctx, child_depth, retry, ply + 1, false);
            }
            if score > window.alpha && score < window.beta {
                score = -negamax(
                    ctx,
                    depth.saturating_sub(1),
                    window.flipped(),
                    ply + 1,
                    true,
                );
            }
        }
        ctx.state.pop_repetition();
        ctx.board.unmake_move(undo);
        if score > best_value {
            best_value = score;
            best_move = Some(mv);
        }
        window.alpha = window.alpha.max(score);
        if window.alpha >= window.beta {
            if !is_capture {
                ctx.state.record_killer(ply, mv);
                ctx.state.record_history(mv, depth);
            }
            break;
        }
    }

    let bound = if best_value <= alpha_orig {
        Bound::Upper
    } else if best_value >= beta_orig {
        Bound::Lower
    } else {
        Bound::Exact
    };
    let store_value = to_tt_score(best_value, ply);
    ctx.tt.store(key, depth, store_value, bound, best_move);
    best_value
}

fn quiescence(ctx: &mut SearchContext, mut window: SearchWindow, ply: usize) -> i32 {
    if ctx.state.should_stop() {
        return window.alpha;
    }
    ctx.stats.record_node();
    if ctx.state.is_threefold(ctx.board.hash()) {
        return 0;
    }
    if ctx.board.halfmove_clock >= 100 {
        return 0;
    }
    if let Some(score) = tablebase::probe_score(ctx.board, ply) {
        return score;
    }

    let stand_pat = evaluate(ctx);
    if stand_pat >= window.beta {
        return stand_pat;
    }
    if window.alpha < stand_pat {
        window.alpha = stand_pat;
    }

    let mut moves = MoveList::new();
    ctx.board.legal_moves_into(&mut moves);
    let tt_move = ctx
        .tt
        .probe(ctx.board.hash())
        .and_then(|entry| entry.best_move);
    let killers = ctx.state.killer_moves(ply);
    order_moves(ctx.board, &mut moves, None, tt_move, killers, ctx.state);

    for &mv in moves.as_slice() {
        if ctx.board.piece_at(mv.to).is_none() {
            continue;
        }
        if ctx.board.static_exchange_eval(mv) < -SEE_RANGE {
            continue;
        }
        let undo = ctx.board.make_move(mv);
        let irreversible = undo.captured_piece.is_some()
            || undo.moving_piece.kind == PieceKind::Pawn;
        ctx.state.push_repetition(ctx.board.hash(), irreversible);
        let score =
            -quiescence(ctx, SearchWindow::new(-window.beta, -window.alpha), ply + 1);
        ctx.state.pop_repetition();
        ctx.board.unmake_move(undo);

        if score >= window.beta {
            return window.beta;
        }
        window.alpha = window.alpha.max(score);
    }

    window.alpha
}

fn immediate_mate_move(board: &mut Board, root_moves: &MoveList) -> Option<Move> {
    for &mv in root_moves.as_slice() {
        let undo = board.make_move(mv);
        let mut replies = MoveList::new();
        board.legal_moves_into(&mut replies);
        let is_mate = replies.is_empty() && board.is_in_check(board.active_color);
        board.unmake_move(undo);
        if is_mate {
            return Some(mv);
        }
    }

    None
}

fn evaluate(ctx: &SearchContext) -> i32 {
    ctx.nnue_runner.eval(ctx.board).unwrap_or(-MATE_VALUE) + mop_up_score(ctx.board)
}

fn color_index(color: crate::chess::Color) -> usize {
    match color {
        crate::chess::Color::White => 0,
        crate::chess::Color::Black => 1,
    }
}

fn opponent_color(color: crate::chess::Color) -> crate::chess::Color {
    match color {
        crate::chess::Color::White => crate::chess::Color::Black,
        crate::chess::Color::Black => crate::chess::Color::White,
    }
}

fn king_distance(a: crate::chess::Square, b: crate::chess::Square) -> i32 {
    let rank_distance = (a.rank() as i32 - b.rank() as i32).abs();
    let file_distance = (a.file() as i32 - b.file() as i32).abs();
    rank_distance.max(file_distance)
}

fn edge_distance_bonus(square: crate::chess::Square) -> i32 {
    let rank = square.rank() as i32;
    let file = square.file() as i32;
    let rank_edge = rank.min(7 - rank);
    let file_edge = file.min(7 - file);
    3 - rank_edge.min(file_edge)
}

fn mop_up_score(board: &Board) -> i32 {
    let mut material = [0; 2];
    let mut kings = [None, None];

    for idx in 0..64 {
        let square = crate::chess::Square::from_index(idx);
        if let Some(piece) = board.piece_at(square) {
            let color_idx = color_index(piece.color);
            if piece.kind == PieceKind::King {
                kings[color_idx] = Some(square);
            } else {
                material[color_idx] += piece_value(piece.kind);
            }
        }
    }

    let side = board.active_color;
    let opponent = opponent_color(side);
    let side_idx = color_index(side);
    let opponent_idx = color_index(opponent);
    let side_advantage = material[side_idx] - material[opponent_idx];

    let (winning_idx, losing_idx, sign) = if side_advantage >= 500 {
        (side_idx, opponent_idx, 1)
    } else if side_advantage <= -500 {
        (opponent_idx, side_idx, -1)
    } else {
        return 0;
    };

    let Some(winning_king) = kings[winning_idx] else {
        return 0;
    };
    let Some(losing_king) = kings[losing_idx] else {
        return 0;
    };

    let edge_bonus = edge_distance_bonus(losing_king) * 28;
    let king_closeness = (7 - king_distance(winning_king, losing_king)) * 12;
    let progress_bonus = (100 - board.halfmove_clock.min(100) as i32) / 4;

    sign * (edge_bonus + king_closeness + progress_bonus)
}

fn order_moves(
    board: &Board,
    moves: &mut MoveList,
    preferred: Option<Move>,
    tt_move: Option<Move>,
    killer_moves: [Option<Move>; 2],
    state: &SearchState,
) {
    let slice = moves.as_mut_slice();
    slice.sort_by_key(|mv| {
        move_score(board, *mv, preferred, tt_move, killer_moves, state)
    });
    slice.reverse();
}

fn move_score(
    board: &Board,
    mv: Move,
    preferred: Option<Move>,
    tt_move: Option<Move>,
    killer_moves: [Option<Move>; 2],
    state: &SearchState,
) -> i32 {
    const PREFERRED_BONUS: i32 = 1_000_000;
    const PROMOTION_BONUS: i32 = 50_000;
    const CAPTURE_BONUS: i32 = 10_000;
    const TT_BONUS: i32 = 2_000_000;
    const KILLER_BONUS: i32 = 30_000;

    let mut score = 0;
    if tt_move == Some(mv) {
        score += TT_BONUS;
    }

    if preferred == Some(mv) {
        score += PREFERRED_BONUS;
    }

    if let Some(killer) = killer_moves[0] {
        if killer == mv {
            score += KILLER_BONUS;
        }
    }
    if let Some(killer) = killer_moves[1] {
        if killer == mv {
            score += KILLER_BONUS / 2;
        }
    }

    if let Some(moving) = board.piece_at(mv.from) {
        if moving.kind == PieceKind::Pawn && (mv.to.rank() == 0 || mv.to.rank() == 7)
        {
            score += PROMOTION_BONUS;
        }
        if let Some(captured) = board.piece_at(mv.to) {
            score +=
                CAPTURE_BONUS + piece_value(captured.kind) - piece_value(moving.kind);
        }
    }

    score += state.history_score(mv);

    score
}
