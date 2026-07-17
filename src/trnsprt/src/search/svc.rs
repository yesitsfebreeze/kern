use super::dto::{
	EntityKindLite, NeighborsReq, NeighborsRes, PreviewReq, PreviewRes, SearchReq, SearchRes,
};

crate::service! {
		pub trait SearchSvc {
				async fn search(req: SearchReq) -> SearchRes;
				async fn neighbors(req: NeighborsReq) -> NeighborsRes;
				async fn preview(req: PreviewReq) -> PreviewRes;
				async fn kinds() -> Vec<EntityKindLite>;
		}
}
