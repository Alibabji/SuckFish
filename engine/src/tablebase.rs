use crate::chess::{Board, Move, MoveList, Square};
use shakmaty::{fen::Fen, CastlingMode, Chess, Move as SyzygyMove};
use shakmaty_syzygy::{Tablebase, Wdl};
use std::env;
use std::sync::OnceLock;

const TABLEBASE_MAX_PIECES: u32 = 5;
const TABLEBASE_WIN_SCORE: i32 = 29_000;

static TABLEBASE: OnceLock<Option<Tablebase<Chess>>> = OnceLock::new();

fn configured_path() -> Option<String> {
    env::var("SUCKFISH_TB_PATH")
        .ok()
        .or_else(|| env::var("SYZYGY_PATH").ok())
        .filter(|path| !path.trim().is_empty())
}

fn tablebase() -> Option<&'static Tablebase<Chess>> {
    TABLEBASE
        .get_or_init(|| {
            let path = configured_path()?;
            let mut tables = Tablebase::<Chess>::new();
            match tables.add_directory(&path) {
                Ok(count) if count > 0 => {
                    eprintln!("loaded {count} Syzygy tablebase files from {path}");
                    Some(tables)
                }
                Ok(_) => {
                    eprintln!("no Syzygy tablebase files found in {path}");
                    None
                }
                Err(err) => {
                    eprintln!(
                        "failed to load Syzygy tablebase directory {path}: {err}"
                    );
                    None
                }
            }
        })
        .as_ref()
}

fn to_shakmaty(board: &Board) -> Option<Chess> {
    board
        .fen()
        .parse::<Fen>()
        .ok()?
        .into_position(CastlingMode::Standard)
        .ok()
}

fn from_shakmaty_square(square: shakmaty::Square) -> Square {
    Square::unchecked(
        u32::from(square.rank()) as u8,
        u32::from(square.file()) as u8,
    )
}

fn from_syzygy_move(mv: &SyzygyMove, legal_moves: &MoveList) -> Option<Move> {
    let candidate = match mv {
        SyzygyMove::Normal {
            from,
            to,
            promotion,
            ..
        } => {
            let from = from_shakmaty_square(*from);
            let to = from_shakmaty_square(*to);
            if promotion.is_some() {
                Move::with_promotion(from, to)
            } else {
                Move::new(from, to)
            }
        }
        SyzygyMove::EnPassant { from, to } => {
            Move::new(from_shakmaty_square(*from), from_shakmaty_square(*to))
        }
        SyzygyMove::Castle { .. } | SyzygyMove::Put { .. } => return None,
    };

    legal_moves
        .as_slice()
        .contains(&candidate)
        .then_some(candidate)
}

fn wdl_score_for_side_to_move(wdl: Wdl, ply: usize) -> i32 {
    match wdl {
        Wdl::Win => TABLEBASE_WIN_SCORE - ply as i32,
        Wdl::CursedWin => TABLEBASE_WIN_SCORE - 1_000 - ply as i32,
        Wdl::Draw => 0,
        Wdl::BlessedLoss => -TABLEBASE_WIN_SCORE + 1_000 + ply as i32,
        Wdl::Loss => -TABLEBASE_WIN_SCORE + ply as i32,
    }
}

pub fn probe_score(board: &Board, ply: usize) -> Option<i32> {
    if board.piece_count() > TABLEBASE_MAX_PIECES {
        return None;
    }

    let tables = tablebase()?;
    let position = to_shakmaty(board)?;
    let wdl = tables.probe_wdl_after_zeroing(&position).ok()?;
    Some(wdl_score_for_side_to_move(wdl, ply))
}

pub fn best_root_move(board: &mut Board) -> Option<Move> {
    if board.piece_count() > TABLEBASE_MAX_PIECES {
        return None;
    }

    tablebase()?;

    let mut moves = MoveList::new();
    board.legal_moves_into(&mut moves);
    if let Some(tables) = tablebase() {
        if let Some(position) = to_shakmaty(board) {
            if let Ok(Some((tb_move, _dtz))) = tables.best_move(&position) {
                if let Some(best_move) = from_syzygy_move(&tb_move, &moves) {
                    return Some(best_move);
                }
            }
        }
    }

    let mut best_move = None;
    let mut best_score = i32::MIN;

    for &mv in moves.as_slice() {
        let undo = board.make_move(mv);
        let score = probe_score(board, 1)
            .map(|score| -score + root_progress_bonus(board, mv));
        board.unmake_move(undo);

        if let Some(score) = score {
            if score > best_score {
                best_score = score;
                best_move = Some(mv);
            }
        }
    }

    best_move
}

fn root_progress_bonus(board_after: &mut Board, mv: Move) -> i32 {
    let mut replies = MoveList::new();
    board_after.legal_moves_into(&mut replies);

    if replies.is_empty() {
        return if board_after.is_in_check(board_after.active_color) {
            900
        } else {
            -10_000
        };
    }

    let mut bonus = 0;
    if mv.is_promotion() {
        bonus += 160;
    }
    if board_after.halfmove_clock == 0 {
        bonus += 120;
    }
    if board_after.is_in_check(board_after.active_color) {
        bonus += 80;
    }

    bonus += (64 - replies.len() as i32).clamp(0, 64);
    bonus += (100 - board_after.halfmove_clock.min(100) as i32) / 3;
    bonus
}
